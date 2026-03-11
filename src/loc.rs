use colored::Colorize;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::render;

/// Per-file line count data.
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

/// Count lines in a single file. Code lines exclude blank lines and comment-only lines.
fn count_lines(path: &Path) -> Option<FileLocStats> {
    let content = std::fs::read_to_string(path).ok()?;
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

    Some(FileLocStats {
        path: path.to_path_buf(),
        total_lines,
        code_lines,
    })
}

/// Count lines for all files and compute aggregate statistics.
pub fn analyze_files(files: &[PathBuf]) -> (Vec<FileLocStats>, Option<AggregateStats>) {
    let mut file_stats: Vec<FileLocStats> = files
        .iter()
        .filter_map(|f| count_lines(f))
        .collect();

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

    println!(
        "┌─────────────┬──────────────┐"
    );
    render::print_stat_row("Total files", &format!("{}", agg.total_files));
    render::print_stat_row("Total LoC", &format!("{}", agg.total_loc));
    render::print_stat_row("Mean", &format!("{:.1}", agg.mean));
    render::print_stat_row("Std Dev", &format!("{:.1}", agg.std_dev));
    render::print_stat_row("Median", &format!("{}", agg.median));
    render::print_stat_row("Min", &format!("{}", agg.min));
    render::print_stat_row("Max", &format!("{}", agg.max));
    println!(
        "└─────────────┴──────────────┘"
    );
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
            "Bar chart: files ranked by code lines (excluding blank lines and comments).",
            "Bar length is proportional to LoC relative to the largest file.",
            "Color gradient: red = top of the ranking (most lines), green = bottom.",
        ]);
        render::guide_ref("size");
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
            format!(
                "  ... and {} more files",
                file_stats.len() - display_count
            )
            .dimmed()
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
        .map(|(directory, code_lines)| DirBreakdownJson { directory, code_lines })
        .collect();

    let output = LocJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        files: files_json,
        aggregate,
        directory_breakdown,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
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
    let pos = node.children.iter().position(|c| !c.is_leaf() && c.name == *dir_name);
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

            lines.push(format!("│{}{}{}│", colored_name, dots.dimmed(), colored_loc));
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

