use colored::Colorize;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use crate::{ast_parser, datapaths, file_metrics, flow, render};
/// Per-file line count data.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct FileLocStats {
    pub path: PathBuf,
    pub total_lines: usize,
    pub code_lines: usize,
}

/// Aggregate statistics across all files.
pub struct AggregateStats {
    pub total_files: usize,
    pub total_loc: usize,
    pub mean: f64,
    pub std_dev: f64,
    pub median: usize,
    pub min: usize,
    pub max: usize,
}

#[derive(Serialize)]
struct FileLocJson {
    path: String,
    total_lines: usize,
    code_lines: usize,
}

#[derive(Serialize)]
struct AggregateStatsJson {
    total_files: usize,
    total_loc: usize,
    mean: f64,
    std_dev: f64,
    median: usize,
    min: usize,
    max: usize,
}

#[derive(Serialize)]
struct DirBreakdownJson {
    directory: String,
    code_lines: usize,
}

#[derive(Serialize)]
struct LocJson {
    cstat_version: String,
    files: Vec<FileLocJson>,
    aggregate: Option<AggregateStatsJson>,
    directory_breakdown: Vec<DirBreakdownJson>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileLineSpan {
    pub function: String,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileProjectedLineReachabilityReport {
    pub file: String,
    pub total_lines: usize,
    pub code_lines: usize,
    pub projected_reachable_lines: usize,
    pub projected_unreachable_lines: usize,
    pub reachable_spans: Vec<FileLineSpan>,
    pub unreachable_spans: Vec<FileLineSpan>,
}

/// Classify a single trimmed line as code or non-code, tracking block comment state.
/// Returns `(is_code, new_in_block_comment)`.
fn classify_line(trimmed: &str, in_block_comment: bool) -> (bool, bool) {
    if in_block_comment {
        if let Some(pos) = trimmed.find("*/") {
            let after = trimmed[pos + 2..].trim();
            let is_code = !after.is_empty() && !after.starts_with("//");
            return (is_code, false);
        }
        return (false, true);
    }

    if trimmed.is_empty() || trimmed.starts_with("//") {
        return (false, false);
    }

    if trimmed.starts_with("/*") {
        if !trimmed.contains("*/") {
            return (false, true);
        }
        // Single-line block comment like /* ... */
        let after_close = if let Some(pos) = trimmed.find("*/") {
            trimmed[pos + 2..].trim()
        } else {
            ""
        };
        if after_close.is_empty() {
            return (false, false);
        }
    }

    (true, false)
}

/// Classify and count total/code lines over an in-memory string.
fn count_string_lines(content: &str) -> (usize, usize) {
    let total_lines = content.lines().count();
    let mut code_lines = 0usize;
    let mut in_block_comment = false;
    for line in content.lines() {
        let (is_code, new_in_block) = classify_line(line.trim(), in_block_comment);
        in_block_comment = new_in_block;
        if is_code {
            code_lines += 1;
        }
    }
    (total_lines, code_lines)
}

/// Raw textual line count of a file, with no rustfmt normalization.
///
/// Used by projected-line reachability, whose spans are real source line numbers
/// tied to static reachability and coverage — normalizing there would desync the
/// reported spans from the reported line counts.
fn count_raw_lines(path: &Path) -> Option<FileLocStats> {
    let content = std::fs::read_to_string(path).ok()?;
    let (total_lines, code_lines) = count_string_lines(&content);
    Some(FileLocStats {
        path: path.to_path_buf(),
        total_lines,
        code_lines,
    })
}

/// Count lines in a single file, rustfmt-normalized first so line-packed source
/// cannot deflate LOC. Formats a temp copy of this one file; the working tree is
/// never modified. Falls back to raw counting when rustfmt is unavailable or the
/// file fails to format.
pub fn count_file_lines(path: &Path) -> Option<FileLocStats> {
    let key = path.to_path_buf();
    let mut map = count_file_lines_batch(std::slice::from_ref(&key));
    map.remove(&key)
}

/// Detect the Rust edition for `file` from the nearest ancestor `Cargo.toml`,
/// memoized per manifest. Defaults to "2021" when no manifest or edition key is
/// found. A wrong guess only causes a rustfmt parse failure, which falls back to
/// raw counting — so the worst case is losing normalization on that one file.
fn rustfmt_edition_for(file: &Path) -> String {
    static CACHE: LazyLock<Mutex<HashMap<PathBuf, String>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    let mut dir = file.parent();
    while let Some(d) = dir {
        let cargo = d.join("Cargo.toml");
        if cargo.is_file() {
            if let Some(e) = CACHE.lock().ok().and_then(|g| g.get(&cargo).cloned()) {
                return e;
            }
            let e = read_edition(&cargo).unwrap_or_else(|| "2021".to_string());
            if let Ok(mut guard) = CACHE.lock() {
                guard.insert(cargo.clone(), e.clone());
            }
            return e;
        }
        dir = d.parent();
    }
    "2021".to_string()
}

/// Scan a `Cargo.toml` for an `edition = "YYYY"` line (2015/2018/2021/2024).
fn read_edition(cargo: &Path) -> Option<String> {
    let text = std::fs::read_to_string(cargo).ok()?;
    for line in text.lines() {
        let t = line.trim();
        let rest = t.strip_prefix("edition")?.trim_start();
        let rest = rest.strip_prefix('=')?.trim();
        let v = rest.trim_matches('"');
        if matches!(v, "2015" | "2018" | "2021" | "2024") {
            return Some(v.to_string());
        }
    }
    None
}

/// Create a unique temp directory for rustfmt copies.
fn make_temp_dir() -> Option<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cstat-fmt-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).ok().map(|_| dir)
}

/// Count lines for every file, rustfmt-normalizing them first.
///
/// All measured files are copied into one temp directory and formatted in a
/// SINGLE `rustfmt` invocation (chunked only to bound argument length), so the
/// cost is roughly constant in file count rather than one spawn per file. The
/// working tree is never modified: counts come from the temp copies, removed
/// afterward.
///
/// rustfmt leaves any file it cannot parse as its original copy, so unreadable
/// or edition-mismatched files automatically fall back to raw textual counting.
/// If rustfmt is missing or the temp directory cannot be created, every file
/// falls back to raw counting.
pub fn count_file_lines_batch(files: &[PathBuf]) -> HashMap<PathBuf, FileLocStats> {
    let mut out: HashMap<PathBuf, FileLocStats> = HashMap::new();
    if files.is_empty() {
        return out;
    }

    // Read every source into memory (None == unreadable / non-UTF-8).
    let sources: Vec<Option<String>> = files.iter().map(|f| std::fs::read_to_string(f).ok()).collect();

    let tmp_dir = match make_temp_dir() {
        Some(d) => d,
        None => {
            for (file, content) in files.iter().zip(sources.iter()) {
                if let Some(c) = content {
                    let (t, cl) = count_string_lines(c);
                    out.insert(
                        file.clone(),
                        FileLocStats {
                            path: file.clone(),
                            total_lines: t,
                            code_lines: cl,
                        },
                    );
                }
            }
            return out;
        }
    };

    // Copy each readable file into the temp dir, keyed by index.
    let edition = rustfmt_edition_for(&files[0]);
    let mut tmp_paths: HashMap<usize, PathBuf> = HashMap::new();
    for (i, content) in sources.iter().enumerate() {
        if let Some(c) = content {
            let tmp = tmp_dir.join(format!("{i}.rs"));
            if std::fs::write(&tmp, c).is_ok() {
                tmp_paths.insert(i, tmp);
            }
        }
    }

    // Format all copies. Chunk to keep the argument list well under ARG_MAX.
    let all_tmp: Vec<PathBuf> = tmp_paths.values().cloned().collect();
    const CHUNK: usize = 512;
    for chunk in all_tmp.chunks(CHUNK) {
        let mut cmd = Command::new("rustfmt");
        cmd.arg("--edition").arg(&edition);
        for tmp in chunk {
            cmd.arg(tmp);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Exit status is intentionally ignored: rustfmt formats the files it can
        // and leaves unparseable ones as their original copy (raw fallback).
        let _ = cmd.status();
    }

    // Count from the (now possibly formatted) temp copies.
    for (i, file) in files.iter().enumerate() {
        let counted = match tmp_paths.get(&i) {
            Some(tmp) => match std::fs::read_to_string(tmp) {
                Ok(formatted) => {
                    let (t, cl) = count_string_lines(&formatted);
                    FileLocStats {
                        path: file.clone(),
                        total_lines: t,
                        code_lines: cl,
                    }
                }
                // Temp copy unreadable: fall back to the original source.
                Err(_) => match &sources[i] {
                    Some(c) => {
                        let (t, cl) = count_string_lines(c);
                        FileLocStats {
                            path: file.clone(),
                            total_lines: t,
                            code_lines: cl,
                        }
                    }
                    None => continue,
                },
            },
            // No temp copy was written (source unreadable): skip, matching the
            // old behavior of dropping unreadable files.
            None => continue,
        };
        out.insert(file.clone(), counted);
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);
    out
}

fn code_lines_in_spans(path: &Path, spans: &[(usize, usize)]) -> HashSet<usize> {
    if spans.is_empty() {
        return HashSet::new();
    }

    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return HashSet::new(),
    };

    let mut covered = HashSet::new();
    let mut in_block_comment = false;

    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        let (is_code, new_in_block) = classify_line(line.trim(), in_block_comment);
        in_block_comment = new_in_block;
        if !is_code {
            continue;
        }

        if spans
            .iter()
            .any(|(start, end)| line_number >= *start && line_number <= *end)
        {
            covered.insert(line_number);
        }
    }

    covered
}

pub fn analyze_file_projected_line_reachability(
    file: &Path,
    file_functions: &file_metrics::FileFunctionIndex,
    reachable_functions: &HashSet<String>,
    file_display_path: &str,
) -> Option<FileProjectedLineReachabilityReport> {
    let file_stats = count_raw_lines(file)?;
    let mut reachable_spans = Vec::new();
    let mut unreachable_spans = Vec::new();
    let mut production_ranges = Vec::new();
    let mut reachable_ranges = Vec::new();

    for function in &file_functions.functions {
        if function.is_test
            || function.is_bench
            || function.is_test_support
            || function.is_binary_wrapper_main
        {
            continue;
        }

        production_ranges.push((function.line_start, function.line_end));
        let span = FileLineSpan {
            function: function.display_name.clone(),
            line_start: function.line_start,
            line_end: function.line_end,
        };

        if reachable_functions.contains(&function.graph_name) {
            reachable_ranges.push((function.line_start, function.line_end));
            reachable_spans.push(span);
        } else {
            unreachable_spans.push(span);
        }
    }

    let code_lines = code_lines_in_spans(file, &production_ranges).len();
    let projected_reachable_lines = code_lines_in_spans(file, &reachable_ranges).len();
    let projected_unreachable_lines = code_lines.saturating_sub(projected_reachable_lines);

    Some(FileProjectedLineReachabilityReport {
        file: file_display_path.to_string(),
        total_lines: file_stats.total_lines,
        code_lines,
        projected_reachable_lines,
        projected_unreachable_lines,
        reachable_spans,
        unreachable_spans,
    })
}

/// Count lines for all files and compute aggregate statistics.
pub fn analyze_files(files: &[PathBuf]) -> (Vec<FileLocStats>, Option<AggregateStats>) {
    let stats = count_file_lines_batch(files);
    let mut file_stats: Vec<FileLocStats> = files.iter().filter_map(|f| stats.get(f).cloned()).collect();

    // Sort descending by code lines
    file_stats.sort_by(|a, b| b.code_lines.cmp(&a.code_lines));

    if file_stats.is_empty() {
        return (file_stats, None);
    }

    let total_files = file_stats.len();
    let total_loc: usize = file_stats.iter().map(|f| f.code_lines).sum();
    let mean = total_loc as f64 / total_files as f64;

    let variance: f64 = file_stats
        .iter()
        .map(|f| {
            let diff = f.code_lines as f64 - mean;
            diff * diff
        })
        .sum::<f64>()
        / total_files as f64;
    let std_dev = variance.sqrt();

    let mut sorted_loc: Vec<usize> = file_stats.iter().map(|f| f.code_lines).collect();
    sorted_loc.sort();
    let median = if total_files % 2 == 0 {
        (sorted_loc[total_files / 2 - 1] + sorted_loc[total_files / 2]) / 2
    } else {
        sorted_loc[total_files / 2]
    };

    let min = sorted_loc[0];
    let max = sorted_loc[sorted_loc.len() - 1];

    let agg = AggregateStats {
        total_files,
        total_loc,
        mean,
        std_dev,
        median,
        min,
        max,
    };

    (file_stats, Some(agg))
}

/// Strip the common project prefix from paths for display.
fn strip_prefix<'a>(path: &'a Path, base: &Path) -> &'a Path {
    path.strip_prefix(base).unwrap_or(path)
}

/// Render a bar chart of LoC per file.
fn render_loc_bar_chart(
    display_stats: &[FileLocStats],
    labels: &[String],
    max_label_len: usize,
    max_loc: usize,
    bar_budget: usize,
) {
    let display_count = display_stats.len();
    for (i, (stat, label)) in display_stats.iter().zip(labels.iter()).enumerate() {
        let ratio = i as f64 / display_count.max(1) as f64;
        let bar_len = if max_loc > 0 {
            (stat.code_lines as f64 / max_loc as f64 * bar_budget as f64).ceil() as usize
        } else {
            0
        }
        .max(if stat.code_lines > 0 { 1 } else { 0 });

        let bar_str = "█".repeat(bar_len);
        let colored_bar = render::bar_color(&bar_str, ratio);

        println!(
            "{:>width$} │ {}{} {}",
            label.dimmed(),
            colored_bar,
            " ".repeat(bar_budget.saturating_sub(bar_len)),
            format!("{}", stat.code_lines).bold(),
            width = max_label_len,
        );
    }
}

/// Render the statistics summary box.
fn render_loc_stats_box(agg: &AggregateStats, verbose: bool) {
    println!(
        "\n{}",
        "── Statistics ─────────────────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Descriptive statistics across all analyzed files.",
            "Mean = average LoC per file. Std Dev = spread around the mean.",
            "Median = middle value when files are sorted by LoC (robust to outliers).",
        ]);
    }

    println!("┌─────────────┬──────────────┐");
    render::print_stat_row("Total files", &format!("{}", agg.total_files));
    render::print_stat_row("Total LoC", &format!("{}", agg.total_loc));
    render::print_stat_row("Mean", &format!("{:.1}", agg.mean));
    render::print_stat_row("Std Dev", &format!("{:.1}", agg.std_dev));
    render::print_stat_row("Median", &format!("{}", agg.median));
    render::print_stat_row("Min", &format!("{}", agg.min));
    render::print_stat_row("Max", &format!("{}", agg.max));
    println!("└─────────────┴──────────────┘");
}

/// Render the full LoC analysis for the `loc` subcommand.
pub fn render_loc(files: &[PathBuf], project_path: &Path, top_n: Option<usize>, verbose: bool) {
    let (file_stats, agg) = analyze_files(files);

    if file_stats.is_empty() {
        println!("{}", "No source files found.".yellow());
        return;
    }

    let agg = agg.unwrap();
    let term_w = render::terminal_width();

    // Determine how many files to show
    let display_count = top_n.unwrap_or(file_stats.len()).min(file_stats.len());
    let display_stats = &file_stats[..display_count];

    // Find longest label for alignment
    let labels: Vec<String> = display_stats
        .iter()
        .map(|f| format!("{}", strip_prefix(&f.path, project_path).display()))
        .collect();
    let max_label_len = labels.iter().map(|l| l.len()).max().unwrap_or(0);
    let max_loc = display_stats
        .iter()
        .map(|f| f.code_lines)
        .max()
        .unwrap_or(1);
    let loc_digits = format!("{}", max_loc).len();

    let bar_budget = term_w
        .saturating_sub(max_label_len)
        .saturating_sub(3)
        .saturating_sub(loc_digits)
        .saturating_sub(2)
        .max(10);

    // Header
    println!(
        "\n{}",
        "── Lines of Code ────────────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Bar chart: files ranked by code_lines.",
            "code_lines excludes blank lines, // comment-only lines, and block-comment-only regions.",
            "Line classification is static text scanning, not semantic Rust parsing.",
            "Bar length is proportional to code_lines relative to the largest file.",
            "Color gradient: red = top of the ranking (most lines), green = bottom.",
            "Full loc reference: cstat loc --explain.",
        ]);
    }

    println!(
        "{} files analyzed, {} total code lines\n",
        format!("{}", agg.total_files).bold(),
        format!("{}", agg.total_loc).bold(),
    );

    // Bar chart
    render_loc_bar_chart(display_stats, &labels, max_label_len, max_loc, bar_budget);

    if display_count < file_stats.len() {
        println!(
            "{}",
            format!("  ... and {} more files", file_stats.len() - display_count).dimmed()
        );
    }

    // Stats box
    render_loc_stats_box(&agg, verbose);

    // Treemap visualization
    render_treemap(&file_stats, project_path, verbose);
}

/// Render LoC analysis as JSON.
pub fn render_loc_json(files: &[PathBuf], project_path: &Path) {
    let (file_stats, agg) = analyze_files(files);

    let files_json: Vec<FileLocJson> = file_stats
        .iter()
        .map(|f| FileLocJson {
            path: strip_prefix(&f.path, project_path).display().to_string(),
            total_lines: f.total_lines,
            code_lines: f.code_lines,
        })
        .collect();

    let aggregate = agg.map(|a| AggregateStatsJson {
        total_files: a.total_files,
        total_loc: a.total_loc,
        mean: a.mean,
        std_dev: a.std_dev,
        median: a.median,
        min: a.min,
        max: a.max,
    });

    // Directory breakdown
    let mut dir_loc: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for stat in &file_stats {
        let rel = stat.path.strip_prefix(project_path).unwrap_or(&stat.path);
        let components: Vec<_> = rel.components().collect();
        let dir_name = if components.len() <= 1 {
            ".".to_string()
        } else {
            components[0].as_os_str().to_string_lossy().to_string()
        };
        *dir_loc.entry(dir_name).or_insert(0) += stat.code_lines;
    }
    let mut dirs: Vec<(String, usize)> = dir_loc.into_iter().collect();
    dirs.sort_by(|a, b| b.1.cmp(&a.1));
    let directory_breakdown: Vec<DirBreakdownJson> = dirs
        .into_iter()
        .map(|(directory, code_lines)| DirBreakdownJson {
            directory,
            code_lines,
        })
        .collect();

    let output = LocJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        files: files_json,
        aggregate,
        directory_breakdown,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}

pub fn render_loc_file(rs_files: &[PathBuf], project_path: &Path, file: &Path, verbose: bool) {
    let report = analyze_file_projected_line_reachability_from_files(rs_files, project_path, file);
    render_file_projected_line_reachability_report(&report, verbose);
}

pub fn render_loc_file_json(rs_files: &[PathBuf], project_path: &Path, file: &Path) {
    if let Some(report) =
        analyze_file_projected_line_reachability_from_files(rs_files, project_path, file)
    {
        println!("{}", serde_json::to_string(&report).unwrap());
    } else {
        println!("null");
    }
}

pub fn render_loc_explain(json: bool) {
    if json {
        render_loc_explain_json();
    } else {
        render_loc_explain_human();
    }
}

fn render_loc_explain_human() {
    render::section_header("loc reference");
    println!("  Project mode: cstat loc --path .");
    println!("  Project JSON: cstat loc --path . --json");
    println!("  Selected-file mode: cstat loc --path src/lib.rs");
    println!("  Selected-file JSON: cstat loc --path src/lib.rs --json");
    println!("  Machine contract: cstat loc --explain --json");
    println!();
    println!("  code_lines: static textual classification");
    println!("    excludes blank lines, // comment-only lines, and block-comment-only regions");
    println!("    not semantic Rust parsing");
    println!("  project-mode counts: rustfmt-normalized in a temp copy (working tree never modified);");
    println!("    falls back to raw if rustfmt is missing or a file fails to format");
    println!("  selected-file counts: real source lines (spans tie to reachability), not rustfmt-normalized");
    println!();
    println!("  Project JSON fields: cstat_version, files, aggregate, directory_breakdown");
    println!("  Selected-file JSON fields: file, total_lines, code_lines, projected_reachable_lines, projected_unreachable_lines, reachable_spans, unreachable_spans");
}

fn render_loc_explain_json() {
    let output = serde_json::json!({
        "cstat_version": env!("CARGO_PKG_VERSION"),
        "probe": "loc",
        "commands": {
            "project_human": "cstat loc --path .",
            "project_json": "cstat loc --path . --json",
            "selected_file_human": "cstat loc --path src/lib.rs",
            "selected_file_json": "cstat loc --path src/lib.rs --json",
            "explain_human": "cstat loc --explain",
            "explain_json": "cstat loc --explain --json",
        },
        "code_lines": {
            "kind": "static textual classification",
            "excludes": [
                "blank lines",
                "// comment-only lines",
                "block-comment-only regions via simple /* ... */ state tracking",
            ],
            "not": "semantic Rust parsing",
        },
        "project_mode_normalization": {
            "method": "rustfmt each measured file in a throwaway temp copy, then count",
            "edition": "detected from the nearest Cargo.toml",
            "working_tree": "never modified",
            "fallback": "raw textual counting if rustfmt is unavailable or a file fails to format",
        },
        "selected_file_normalization": {
            "method": "none; counts are real source lines because spans tie to static reachability and coverage",
        },
        "project_json_fields": {
            "cstat_version": "cstat version string",
            "files": ["path", "total_lines", "code_lines"],
            "aggregate": ["total_files", "total_loc", "mean", "std_dev", "median", "min", "max"],
            "directory_breakdown": ["directory", "code_lines"],
        },
        "selected_file_json_fields": {
            "file": "selected file path relative to the crate root",
            "total_lines": "physical lines in the selected file",
            "code_lines": "production function-span code lines after exclusions",
            "projected_reachable_lines": "production code lines in statically reachable function spans",
            "projected_unreachable_lines": "production code lines not in statically reachable function spans",
            "reachable_spans": ["function", "line_start", "line_end"],
            "unreachable_spans": ["function", "line_start", "line_end"],
        },
    });

    println!("{}", serde_json::to_string(&output).unwrap());
}

pub fn analyze_file_projected_line_reachability_from_files(
    rs_files: &[PathBuf],
    project_path: &Path,
    file: &Path,
) -> Option<FileProjectedLineReachabilityReport> {
    let symbols = ast_parser::parse_project(rs_files);
    let graph = flow::build_call_graph(rs_files, project_path);
    let file_functions =
        file_metrics::selected_file_functions(&symbols, &graph, project_path, file);
    let file_display_path = relative_path_string(project_path, file);
    let static_reachability =
        datapaths::analyze_file_static_reachability(&graph, &file_functions, &file_display_path);
    let reachable_functions: HashSet<String> = static_reachability
        .reachable_functions
        .iter()
        .cloned()
        .collect();
    analyze_file_projected_line_reachability(
        file,
        &file_functions,
        &reachable_functions,
        &file_display_path,
    )
}

pub fn render_file_projected_line_reachability_report(
    report: &Option<FileProjectedLineReachabilityReport>,
    verbose: bool,
) {
    render::section_header("Projected line reachability");

    if verbose {
        render::verbose_block(&[
            "Physical lines count every source line in the selected file.",
            "code_lines is production function-span code after excluding tests, benches, test-support helpers, and selected binary wrapper main().",
            "Line classification excludes blank lines, // comment-only lines, and block-comment-only regions with simple text scanning.",
            "Projected reachable lines are production code lines inside functions statically reached from project tests/benches.",
            "Full loc reference: cstat loc --explain.",
        ]);
    }

    let Some(report) = report else {
        println!("  {}", "Unable to read selected file.".yellow());
        return;
    };

    let pct = if report.code_lines == 0 {
        0.0
    } else {
        report.projected_reachable_lines as f64 / report.code_lines as f64 * 100.0
    };

    println!("  file {}", report.file.bold());
    println!("  physical lines {}", report.total_lines);
    println!("  production code lines {}", report.code_lines);
    println!(
        "  statically reachable lines {} / {} ({:.1}%)",
        report.projected_reachable_lines, report.code_lines, pct
    );
    println!(
        "  not statically reachable lines {}",
        report.projected_unreachable_lines
    );

    println!();
    println!("  {}", "reachable function spans:".bold());
    if report.reachable_spans.is_empty() {
        println!("  {}", "<none>".dimmed());
    } else {
        for span in &report.reachable_spans {
            println!(
                "  {} lines {}-{}",
                span.function, span.line_start, span.line_end
            );
        }
    }

    println!();
    println!("  {}", "unreachable function spans:".bold().yellow());
    if report.unreachable_spans.is_empty() {
        println!("  {}", "<none>".dimmed());
    } else {
        for span in &report.unreachable_spans {
            println!(
                "  {} lines {}-{}",
                span.function, span.line_start, span.line_end
            );
        }
    }
}

fn relative_path_string(project_path: &Path, path: &Path) -> String {
    path.strip_prefix(project_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

// ── Treemap Visualization ────────────────────────────────────────────

/// A node in the directory tree used for treemap rendering.
struct TreeNode {
    name: String,
    loc: usize,
    children: Vec<TreeNode>,
}

impl TreeNode {
    fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

/// Build a directory tree from file stats.
fn build_tree(file_stats: &[FileLocStats], project_path: &Path) -> TreeNode {
    let mut root = TreeNode {
        name: project_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string()),
        loc: 0,
        children: Vec::new(),
    };

    for stat in file_stats {
        let rel = strip_prefix(&stat.path, project_path);
        let components: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();

        insert_into_tree(&mut root, &components, stat.code_lines);
    }

    // Compute aggregate LoC for directories
    compute_loc(&mut root);
    root
}

/// Insert a file path into the tree.
fn insert_into_tree(node: &mut TreeNode, parts: &[String], loc: usize) {
    if parts.is_empty() {
        return;
    }
    if parts.len() == 1 {
        // Leaf file
        node.children.push(TreeNode {
            name: parts[0].clone(),
            loc,
            children: Vec::new(),
        });
        return;
    }
    // Find or create directory child
    let dir_name = &parts[0];
    let pos = node
        .children
        .iter()
        .position(|c| !c.is_leaf() && c.name == *dir_name);
    if let Some(idx) = pos {
        insert_into_tree(&mut node.children[idx], &parts[1..], loc);
    } else {
        let mut dir_node = TreeNode {
            name: dir_name.clone(),
            loc: 0,
            children: Vec::new(),
        };
        insert_into_tree(&mut dir_node, &parts[1..], loc);
        node.children.push(dir_node);
    }
}

/// Recursively compute LoC sums for directory nodes.
fn compute_loc(node: &mut TreeNode) -> usize {
    if node.is_leaf() {
        return node.loc;
    }
    let mut total = 0;
    for child in &mut node.children {
        total += compute_loc(child);
    }
    node.loc = total;
    total
}

/// Color text based on LoC proportion: largest = red, smallest = cyan.
fn treemap_color(text: &str, ratio: f64) -> String {
    if ratio > 0.5 {
        text.red().bold().to_string()
    } else if ratio > 0.25 {
        text.yellow().to_string()
    } else if ratio > 0.1 {
        text.green().to_string()
    } else {
        text.cyan().to_string()
    }
}

/// Render the treemap as nested Unicode boxes.
pub fn render_treemap(file_stats: &[FileLocStats], project_path: &Path, verbose: bool) {
    if file_stats.is_empty() {
        return;
    }

    let tree = build_tree(file_stats, project_path);
    let term_w = render::terminal_width().min(120);

    println!(
        "\n{}",
        "── LoC Treemap ────────────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Nested box visualization of directory/file structure.",
            "Box size is proportional to LoC. Directories contain their children.",
            "Color = proportion of total project LoC: red = dominant (>50%),",
            "yellow = significant (>25%), green = moderate (>10%), cyan = small.",
        ]);
    }

    let total_loc = tree.loc.max(1);
    let lines = render_box_lines(&tree, term_w, total_loc);
    for line in &lines {
        println!("{}", line);
    }
    println!();
}

/// Render a tree node as a box, returning lines. Children are nested inside with │ borders.
fn render_box_lines(node: &TreeNode, width: usize, total_loc: usize) -> Vec<String> {
    let mut lines = Vec::new();

    if width < 8 {
        return lines;
    }

    let inner_w = width.saturating_sub(2); // space between │ and │
    let ratio = node.loc as f64 / total_loc as f64;

    // Top border with label
    let label = if node.is_leaf() {
        format!(" {} ({} loc) ", node.name, node.loc)
    } else {
        format!(" {}/ ({} loc) ", node.name, node.loc)
    };

    let label_colored = treemap_color(&label, ratio);
    let label_vis_len = render::visible_len(&label);
    let border_fill = width.saturating_sub(2).saturating_sub(label_vis_len);
    lines.push(format!("┌{}{}┐", label_colored, "─".repeat(border_fill)));

    if node.is_leaf() {
        lines.push(format!("└{}┘", "─".repeat(width.saturating_sub(2))));
        return lines;
    }

    // Sort children by LoC descending
    let mut children: Vec<&TreeNode> = node.children.iter().collect();
    children.sort_by(|a, b| b.loc.cmp(&a.loc));

    for child in &children {
        let child_ratio = child.loc as f64 / total_loc as f64;

        if child.is_leaf() {
            // Render leaf as a single content line
            let file_text = format!("{} ", child.name);
            let loc_text = format!(" {} loc", child.loc);
            let dots_len = inner_w
                .saturating_sub(file_text.len())
                .saturating_sub(loc_text.len());
            let dots = "·".repeat(dots_len);

            let colored_name = treemap_color(&file_text, child_ratio);
            let colored_loc = treemap_color(&loc_text, child_ratio);

            lines.push(format!(
                "│{}{}{}│",
                colored_name,
                dots.dimmed(),
                colored_loc
            ));
        } else {
            // Render sub-directory as a nested box, then wrap each line in our │ borders
            let child_lines = render_box_lines(child, inner_w, total_loc);
            for cl in &child_lines {
                let vis = render::visible_len(cl);
                let padding = inner_w.saturating_sub(vis);
                lines.push(format!("│{}{}│", cl, " ".repeat(padding)));
            }
        }
    }

    // Bottom border
    lines.push(format!("└{}┘", "─".repeat(width.saturating_sub(2))));
    lines
}
