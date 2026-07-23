use colored::Colorize;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{ast_parser, datapaths, file_metrics, flow, loc, render};

#[derive(Debug, Clone, Serialize)]
pub struct CoverageReport {
    pub cstat_version: String,
    pub project: String,
    pub target: String,
    pub dynamic_line_coverage: DynamicLineCoverage,
    pub branch_coverage: BranchCoverage,
    pub static_reachability: Option<datapaths::FileStaticReachabilityReport>,
    pub projected_line_reachability: Option<loc::FileProjectedLineReachabilityReport>,
    pub tool_status: ToolStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct DynamicLineCoverage {
    pub coverable_lines: usize,
    pub executed_lines: usize,
    pub missing_lines: usize,
    pub executed_line_numbers: Vec<usize>,
    pub missing_line_numbers: Vec<usize>,
    pub percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchCoverage {
    pub total_branch_outcomes: usize,
    pub covered_branch_outcomes: usize,
    pub missing_branch_outcomes: usize,
    pub percent: f64,
    pub sites: Vec<BranchSite>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchSite {
    pub line_start: usize,
    pub column_start: usize,
    pub line_end: usize,
    pub column_end: usize,
    pub expression: String,
    pub true_count: u64,
    pub false_count: u64,
    pub taken: Vec<String>,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub ran_tests: bool,
    pub llvm_cov: Option<String>,
    pub llvm_profdata: Option<String>,
    pub profile_dir: Option<String>,
    pub export_source: String,
    pub branch_instrumentation_enabled: Option<bool>,
}

const COVERAGE_VERBOSE_INTRO: &[&str] = &[
    "Normal mode runs cargo test with Rust source-based coverage flags before reading llvm-cov export data.",
    "Use coverage --no-run with CSTAT_LLVM_COV_EXPORT_JSON to analyze an existing llvm-cov export.",
    "Line coverage is dynamic llvm-cov segment data; branch coverage uses branch records when the Rust/LLVM toolchain emits them.",
    "Production spans exclude tests, benches, test-support helpers, and selected src/bin wrapper main().",
    "Selected-file static/projected sections are static reachability context, not runtime coverage.",
];

pub fn render_coverage(
    rs_files: &[PathBuf],
    project_path: &Path,
    selected_file: Option<&Path>,
    no_run: bool,
    json: bool,
    verbose: bool,
) -> Result<(), String> {
    let symbols = ast_parser::parse_project(rs_files);
    let graph = flow::build_call_graph(rs_files, project_path);
    let filter = file_metrics::production_filter_for_files(
        &symbols,
        &graph,
        project_path,
        rs_files,
        selected_file,
    );

    let export_result = if no_run {
        let export_path = std::env::var("CSTAT_LLVM_COV_EXPORT_JSON").map_err(|_| {
            "CSTAT_LLVM_COV_EXPORT_JSON is required when coverage --no-run is used".to_string()
        });
        export_path.and_then(|export_path| {
            let content = std::fs::read_to_string(&export_path).map_err(|e| {
                format!("failed to read CSTAT_LLVM_COV_EXPORT_JSON={export_path}: {e}")
            })?;
            Ok((
                content,
                ToolStatus {
                    ran_tests: false,
                    llvm_cov: None,
                    llvm_profdata: None,
                    profile_dir: None,
                    branch_instrumentation_enabled: None,
                    export_source: export_path,
                },
            ))
        })
    } else {
        collect_coverage_export(project_path, selected_file)
    };
    let (export_json, mut status) = match export_result {
        Ok(result) => result,
        Err(message) => {
            if verbose && !json {
                render_coverage_verbose_intro();
            }
            return Err(message);
        }
    };

    let mut report = analyze_export_json(&export_json, project_path, selected_file, &filter)?;
    report.tool_status = status.clone();

    if let Some(file) = selected_file {
        let file_display_path = relative_path_string(project_path, file);
        let file_functions =
            file_metrics::selected_file_functions(&symbols, &graph, project_path, file);
        let static_reachability = datapaths::analyze_file_static_reachability(
            &graph,
            &file_functions,
            &file_display_path,
        );
        let reachable: std::collections::HashSet<String> = static_reachability
            .reachable_functions
            .iter()
            .cloned()
            .collect();
        let projected = loc::analyze_file_projected_line_reachability(
            file,
            &file_functions,
            &reachable,
            &file_display_path,
        );
        report.static_reachability = Some(static_reachability);
        report.projected_line_reachability = projected;
    }

    status = report.tool_status.clone();
    if json {
        println!(
            "{}",
            serde_json::to_string(&report).map_err(|e| e.to_string())?
        );
    } else {
        render_coverage_report(&report, selected_file.is_some(), &status, verbose);
    }
    Ok(())
}

pub fn analyze_export_json(
    export_json: &str,
    project_path: &Path,
    selected_file: Option<&Path>,
    production_filter: &file_metrics::ProductionFilter,
) -> Result<CoverageReport, String> {
    let value: Value =
        serde_json::from_str(export_json).map_err(|e| format!("invalid llvm-cov JSON: {e}"))?;
    let mut coverable = BTreeSet::new();
    let mut executed = BTreeSet::new();
    let mut sites = Vec::new();
    let mut branch_data_present = false;
    let mut source_cache: HashMap<PathBuf, Vec<String>> = HashMap::new();

    for data in value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for file_obj in data
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(filename) = file_obj.get("filename").and_then(Value::as_str) else {
                continue;
            };
            let Some(source_file) = match_export_file(filename, production_filter) else {
                continue;
            };
            if selected_file.is_some_and(|selected| !same_path_or_suffix(selected, &source_file)) {
                continue;
            }

            if let Some(segments) = file_obj.get("segments").and_then(Value::as_array) {
                for segment in segments {
                    let Some(array) = segment.as_array() else {
                        continue;
                    };
                    let line = array.first().and_then(Value::as_u64).unwrap_or(0) as usize;
                    let count = array.get(2).and_then(Value::as_u64).unwrap_or(0);
                    let has_count = array.get(3).and_then(Value::as_bool).unwrap_or(false);
                    let is_gap = array.get(5).and_then(Value::as_bool).unwrap_or(false);
                    if line == 0
                        || is_gap
                        || !has_count
                        || !production_filter.includes_line(&source_file, line)
                    {
                        continue;
                    }
                    coverable.insert((source_file.clone(), line));
                    if count > 0 {
                        executed.insert((source_file.clone(), line));
                    }
                }
            }

            if let Some(branches) = file_obj.get("branches").and_then(Value::as_array) {
                branch_data_present = true;
                for branch in branches {
                    let Some(array) = branch.as_array() else {
                        continue;
                    };
                    let line_start = array.first().and_then(Value::as_u64).unwrap_or(0) as usize;
                    let column_start = array.get(1).and_then(Value::as_u64).unwrap_or(0) as usize;
                    let line_end = array.get(2).and_then(Value::as_u64).unwrap_or(0) as usize;
                    let column_end = array.get(3).and_then(Value::as_u64).unwrap_or(0) as usize;
                    let true_count = array.get(4).and_then(Value::as_u64).unwrap_or(0);
                    let false_count = array.get(5).and_then(Value::as_u64).unwrap_or(0);
                    if line_start == 0 || !production_filter.includes_line(&source_file, line_start)
                    {
                        continue;
                    }
                    let expression = branch_expression(
                        &source_file,
                        line_start,
                        column_start,
                        line_end,
                        column_end,
                        &mut source_cache,
                    );
                    let mut taken = Vec::new();
                    let mut missing = Vec::new();
                    if true_count > 0 {
                        taken.push("true".to_string());
                    } else {
                        missing.push("true".to_string());
                    }
                    if false_count > 0 {
                        taken.push("false".to_string());
                    } else {
                        missing.push("false".to_string());
                    }
                    sites.push(BranchSite {
                        line_start,
                        column_start,
                        line_end,
                        column_end,
                        expression,
                        true_count,
                        false_count,
                        taken,
                        missing,
                    });
                }
            }
        }
    }

    let executed_line_numbers = line_numbers(&executed);
    let missing_set: BTreeSet<(PathBuf, usize)> =
        coverable.difference(&executed).cloned().collect();
    let missing_line_numbers = line_numbers(&missing_set);
    let coverable_lines = coverable.len();
    let executed_lines = executed.len();
    let missing_lines = coverable_lines.saturating_sub(executed_lines);
    let line_percent = percent(executed_lines, coverable_lines);

    sites.sort_by(|a, b| {
        a.line_start
            .cmp(&b.line_start)
            .then(a.column_start.cmp(&b.column_start))
    });
    let total_branch_outcomes = if branch_data_present {
        sites.len() * 2
    } else {
        0
    };
    let covered_branch_outcomes: usize = sites.iter().map(|site| site.taken.len()).sum();
    let missing_branch_outcomes = total_branch_outcomes.saturating_sub(covered_branch_outcomes);
    let branch_percent = percent(covered_branch_outcomes, total_branch_outcomes);

    Ok(CoverageReport {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        project: project_name(project_path),
        target: selected_file
            .map(|file| relative_path_string(project_path, file))
            .unwrap_or_else(|| "project".to_string()),
        dynamic_line_coverage: DynamicLineCoverage {
            coverable_lines,
            executed_lines,
            missing_lines,
            executed_line_numbers,
            missing_line_numbers,
            percent: line_percent,
        },
        branch_coverage: BranchCoverage {
            total_branch_outcomes,
            covered_branch_outcomes,
            missing_branch_outcomes,
            percent: branch_percent,
            sites,
        },
        static_reachability: None,
        projected_line_reachability: None,
        tool_status: ToolStatus {
            ran_tests: false,
            llvm_cov: None,
            llvm_profdata: None,
            profile_dir: None,
            export_source: "provided_json".to_string(),
            branch_instrumentation_enabled: None,
        },
    })
}

fn render_coverage_verbose_intro() {
    render::verbose_block(COVERAGE_VERBOSE_INTRO);
}

fn render_coverage_report(
    report: &CoverageReport,
    selected_file: bool,
    status: &ToolStatus,
    verbose: bool,
) {
    println!(
        "\n{} {}",
        "cstat coverage".bright_cyan().bold(),
        format!("({})", report.target).dimmed()
    );
    println!("  {} {}", "source".cyan(), status.export_source);

    if verbose {
        render_coverage_verbose_intro();
    }

    if selected_file {
        if let Some(static_reachability) = &report.static_reachability {
            datapaths::render_file_static_reachability_report(static_reachability, verbose);
        }
        loc::render_file_projected_line_reachability_report(
            &report.projected_line_reachability,
            verbose,
        );
    }

    render::section_header("Dynamic line coverage");
    if verbose {
        render::verbose_block(&[
            "Coverable/executed lines come from llvm-cov segment counts after cstat's production-span filter.",
            "Missing lines are coverable production lines whose execution count was zero in the export.",
        ]);
    }
    println!(
        "  executed lines {} / {} ({:.1}%)  missing {}",
        report
            .dynamic_line_coverage
            .executed_lines
            .to_string()
            .bold(),
        report.dynamic_line_coverage.coverable_lines,
        report.dynamic_line_coverage.percent,
        report
            .dynamic_line_coverage
            .missing_lines
            .to_string()
            .yellow(),
    );
    if !report.dynamic_line_coverage.missing_line_numbers.is_empty() {
        let lines = report
            .dynamic_line_coverage
            .missing_line_numbers
            .iter()
            .take(30)
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!("  missing lines: {lines}");
    }

    render::section_header("Branch coverage");
    if verbose {
        render::verbose_block(&[
            "Branch coverage counts true/false outcomes from llvm-cov branch records.",
            "When cstat runs coverage itself, Rust branch records require nightly -Z coverage-options=branch.",
            "When branch data is absent, cstat reports the section as unavailable instead of inventing branch counts.",
        ]);
    }
    if report.branch_coverage.total_branch_outcomes == 0 {
        let message = match status.branch_instrumentation_enabled {
            Some(false) => "branch data unavailable: this rustc does not expose nightly -Z coverage-options=branch, so cstat collected line/region coverage only",
            Some(true) => "branch data unavailable: llvm-cov export contained no branch records for included production files",
            None => "branch data unavailable: llvm-cov export contained no branch records for included production files",
        };
        println!("  {}", message.yellow());
        return;
    }
    println!(
        "  branch outcomes {} / {} ({:.1}%)  missing {}",
        report
            .branch_coverage
            .covered_branch_outcomes
            .to_string()
            .bold(),
        report.branch_coverage.total_branch_outcomes,
        report.branch_coverage.percent,
        report
            .branch_coverage
            .missing_branch_outcomes
            .to_string()
            .yellow(),
    );
    for site in &report.branch_coverage.sites {
        if site.missing.is_empty() {
            continue;
        }
        println!(
            "  line {}: {} missing {} (true={}, false={})",
            site.line_start,
            site.expression,
            site.missing.join("/"),
            site.true_count,
            site.false_count,
        );
    }
}

fn cargo_target_dir(project_path: &Path) -> Result<PathBuf, String> {
    let manifest_path = project_path.join("Cargo.toml");
    let output = Command::new("cargo")
        .arg("metadata")
        .arg("--format-version")
        .arg("1")
        .arg("--no-deps")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .output()
        .map_err(|e| {
            format!(
                "failed to run cargo metadata for {}: {e}",
                manifest_path.display()
            )
        })?;
    let stderr = String::from_utf8(output.stderr).map_err(|e| {
        format!(
            "cargo metadata for {} emitted non-utf8 stderr: {e}",
            manifest_path.display()
        )
    })?;
    let stdout = String::from_utf8(output.stdout).map_err(|e| {
        format!(
            "cargo metadata for {} emitted non-utf8 stdout: {e}",
            manifest_path.display()
        )
    })?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata for {} failed with status {}: {}",
            manifest_path.display(),
            output.status,
            stderr.trim()
        ));
    }
    target_dir_from_metadata_json(&stdout, project_path).map_err(|e| {
        format!(
            "cargo metadata for {} returned invalid target_directory: {e}",
            manifest_path.display()
        )
    })
}

fn target_dir_from_metadata_json(
    metadata_json: &str,
    project_path: &Path,
) -> Result<PathBuf, String> {
    let metadata: Value = serde_json::from_str(metadata_json)
        .map_err(|e| format!("failed to parse cargo metadata JSON: {e}"))?;
    let target_directory = metadata
        .get("target_directory")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing string field target_directory".to_string())?;
    let target_dir = PathBuf::from(target_directory);
    if target_dir.is_absolute() {
        Ok(target_dir)
    } else {
        Ok(project_path.join(target_dir))
    }
}

fn coverage_profile_dir(target_dir: &Path) -> PathBuf {
    target_dir.join("cstat-coverage")
}

fn captured_command_output(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    let mut details = String::new();

    if !stdout.trim().is_empty() {
        details.push_str("\n\ncargo test stdout:\n");
        details.push_str(stdout.trim_end());
    }
    if !stderr.trim().is_empty() {
        details.push_str("\n\ncargo test stderr:\n");
        details.push_str(stderr.trim_end());
    }

    details
}

fn collect_coverage_export(
    project_path: &Path,
    selected_file: Option<&Path>,
) -> Result<(String, ToolStatus), String> {
    let target_dir = cargo_target_dir(project_path)?;
    let llvm_cov = find_tool("LLVM_COV", "llvm-cov")?;
    let llvm_profdata = find_tool("LLVM_PROFDATA", "llvm-profdata")?;
    let profile_dir = coverage_profile_dir(&target_dir);
    std::fs::create_dir_all(&profile_dir)
        .map_err(|e| format!("failed to create {}: {e}", profile_dir.display()))?;
    for stale_profraw in files_with_extension(&profile_dir, "profraw")? {
        std::fs::remove_file(&stale_profraw)
            .map_err(|e| format!("failed to remove {}: {e}", stale_profraw.display()))?;
    }
    let stale_profdata = profile_dir.join("cstat.profdata");
    if stale_profdata.exists() {
        std::fs::remove_file(&stale_profdata)
            .map_err(|e| format!("failed to remove {}: {e}", stale_profdata.display()))?;
    }

    let cargo_args = cargo_test_args(project_path, selected_file);
    let branch_instrumentation_enabled = rustc_supports_branch_coverage();
    let existing_rustflags = std::env::var("RUSTFLAGS").ok();
    let rustflags = coverage_rustflags(
        existing_rustflags.as_deref(),
        branch_instrumentation_enabled,
    );
    let mut cargo = Command::new("cargo");
    cargo.args(&cargo_args);
    let output = cargo
        .env("RUSTFLAGS", rustflags)
        .env(
            "LLVM_PROFILE_FILE",
            profile_dir
                .join("%p-%m.profraw")
                .to_string_lossy()
                .to_string(),
        )
        .output()
        .map_err(|e| format!("failed to run cargo test for coverage: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo test for coverage failed with status {}{}",
            output.status,
            captured_command_output(&output.stdout, &output.stderr),
        ));
    }

    let profraws = files_with_extension(&profile_dir, "profraw")?;
    if profraws.is_empty() {
        return Err(format!(
            "no profraw profiles found in {}",
            profile_dir.display()
        ));
    }
    let profdata = profile_dir.join("cstat.profdata");
    let mut merge = Command::new(&llvm_profdata);
    merge.arg("merge").arg("-sparse");
    for profraw in &profraws {
        merge.arg(profraw);
    }
    merge.arg("-o").arg(&profdata);
    let status = merge
        .status()
        .map_err(|e| format!("failed to run llvm-profdata merge: {e}"))?;
    if !status.success() {
        return Err(format!("llvm-profdata merge failed with status {status}"));
    }

    let binaries = discover_test_binaries(&target_dir, project_path, selected_file)?;
    if binaries.is_empty() {
        return Err(format!(
            "no test binaries found under {}",
            target_dir.join("debug/deps").display()
        ));
    }
    let binaries = coverage_enabled_test_binaries(&llvm_cov, &profdata, &binaries, selected_file)?;
    let mut export = Command::new(&llvm_cov);
    export
        .arg("export")
        .arg("-format=text")
        .arg("-instr-profile")
        .arg(&profdata);
    for binary in &binaries {
        export.arg(binary);
    }
    let output = export
        .output()
        .map_err(|e| format!("failed to run llvm-cov export: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "llvm-cov export failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let json = String::from_utf8(output.stdout)
        .map_err(|e| format!("llvm-cov export emitted non-utf8 output: {e}"))?;
    Ok((
        json,
        ToolStatus {
            ran_tests: true,
            llvm_cov: Some(llvm_cov.to_string_lossy().to_string()),
            llvm_profdata: Some(llvm_profdata.to_string_lossy().to_string()),
            profile_dir: Some(profile_dir.to_string_lossy().to_string()),
            export_source: format_command("cargo", &cargo_args),
            branch_instrumentation_enabled: Some(branch_instrumentation_enabled),
        },
    ))
}

fn cargo_test_args(project_path: &Path, selected_file: Option<&Path>) -> Vec<String> {
    let mut args = vec![
        "test".to_string(),
        "--manifest-path".to_string(),
        project_path
            .join("Cargo.toml")
            .to_string_lossy()
            .to_string(),
        "--no-fail-fast".to_string(),
    ];
    if let Some(file) = selected_file {
        args.extend(selected_target_args(project_path, file));
    }
    args
}

fn selected_target_args(project_path: &Path, selected_file: &Path) -> Vec<String> {
    let rel = selected_file
        .strip_prefix(project_path)
        .unwrap_or(selected_file);
    let components: Vec<String> = rel
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect();
    let stem = selected_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("main")
        .to_string();

    if components.len() >= 3 && components[0] == "src" && components[1] == "bin" {
        return vec![
            "--bin".to_string(),
            manifest_target_name(project_path, "bin", selected_file).unwrap_or(stem),
        ];
    }
    if components.as_slice() == ["src", "main.rs"] {
        return vec![
            "--bin".to_string(),
            manifest_target_name(project_path, "bin", selected_file)
                .or_else(|| package_name(project_path))
                .unwrap_or(stem),
        ];
    }
    if components.len() >= 2 && components[0] == "tests" {
        return vec![
            "--test".to_string(),
            manifest_target_name(project_path, "test", selected_file).unwrap_or(stem),
        ];
    }
    if components.len() >= 2 && components[0] == "benches" {
        return vec![
            "--bench".to_string(),
            manifest_target_name(project_path, "bench", selected_file).unwrap_or(stem),
        ];
    }
    if components
        .first()
        .is_some_and(|component| component == "src")
    {
        return vec!["--lib".to_string()];
    }
    Vec::new()
}

fn manifest_target_name(
    project_path: &Path,
    section: &str,
    selected_file: &Path,
) -> Option<String> {
    let manifest = std::fs::read_to_string(project_path.join("Cargo.toml")).ok()?;
    let wanted_header = format!("[[{section}]]");
    let mut in_target = false;
    let mut name: Option<String> = None;
    let mut path: Option<String> = None;

    let flush = |name: &mut Option<String>, path: &mut Option<String>| -> Option<String> {
        let target_path = path.take()?;
        let target_name = name.take()?;
        let manifest_path = project_path.join(target_path);
        if same_path_or_suffix(&manifest_path, selected_file) {
            Some(target_name)
        } else {
            None
        }
    };

    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_target {
                if let Some(target_name) = flush(&mut name, &mut path) {
                    return Some(target_name);
                }
            }
            in_target = trimmed == wanted_header;
            name = None;
            path = None;
            continue;
        }
        if !in_target {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            match key.trim() {
                "name" => name = quoted_toml_value(value),
                "path" => path = quoted_toml_value(value),
                _ => {}
            }
        }
    }
    if in_target {
        flush(&mut name, &mut path)
    } else {
        None
    }
}

fn quoted_toml_value(raw: &str) -> Option<String> {
    let value = raw.trim().split('#').next()?.trim();
    value
        .strip_prefix('"')
        .and_then(|value| value.split_once('"').map(|(inner, _)| inner.to_string()))
}

fn format_command(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().map(|arg| {
            if arg.contains(char::is_whitespace) {
                format!("{arg:?}")
            } else {
                arg.clone()
            }
        }))
        .collect::<Vec<_>>()
        .join(" ")
}

fn coverage_rustflags(existing_rustflags: Option<&str>, enable_branch_coverage: bool) -> String {
    let mut flags = existing_rustflags
        .map(str::trim)
        .filter(|flags| !flags.is_empty())
        .map(ToString::to_string)
        .unwrap_or_default();
    if !flags.is_empty() {
        flags.push(' ');
    }
    flags.push_str("-C instrument-coverage -C link-dead-code");
    if enable_branch_coverage {
        flags.push_str(" -Z coverage-options=branch");
    }
    flags
}

fn rustc_supports_branch_coverage() -> bool {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| std::ffi::OsString::from("rustc"));
    let Ok(output) = Command::new(rustc).arg("-Z").arg("help").output() else {
        return false;
    };
    output.status.success() && String::from_utf8_lossy(&output.stdout).contains("coverage-options")
}

fn find_tool(env_key: &str, binary: &str) -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os(env_key) {
        let path = PathBuf::from(value);
        if path.exists() {
            return Ok(path);
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(binary);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    if let (Ok(sysroot), Ok(host)) = (rustc_sysroot(), rustc_host()) {
        let candidate = Path::new(sysroot.trim())
            .join("lib/rustlib")
            .join(host.trim())
            .join("bin")
            .join(binary);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("LLVM coverage tools unavailable; install rustup component llvm-tools-preview or set LLVM_COV and LLVM_PROFDATA, or use coverage --no-run with CSTAT_LLVM_COV_EXPORT_JSON".to_string())
}

fn rustc_sysroot() -> Result<String, String> {
    let output = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn rustc_host() -> Result<String, String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_string))
        .ok_or_else(|| "rustc -vV did not report host".to_string())
}

fn files_with_extension(dir: &Path, extension: &str) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for entry in
        std::fs::read_dir(dir).map_err(|e| format!("failed to read {}: {e}", dir.display()))?
    {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|ext| ext == extension) {
            out.push(path);
        }
    }
    Ok(out)
}

fn discover_test_binaries(
    target_dir: &Path,
    project_path: &Path,
    selected_file: Option<&Path>,
) -> Result<Vec<PathBuf>, String> {
    let deps = target_dir.join("debug/deps");
    let selected_prefix =
        selected_file.and_then(|file| selected_test_binary_prefix(project_path, file));
    let package_prefix = package_name(project_path).map(|name| name.replace('-', "_"));
    let mut fallback = Vec::new();
    let mut package_preferred = Vec::new();
    let mut selected_preferred = Vec::new();
    for entry in
        std::fs::read_dir(&deps).map_err(|e| format!("failed to read {}: {e}", deps.display()))?
    {
        let path = entry.map_err(|e| e.to_string())?.path();
        if !path.is_file() || !is_executable(&path) {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if selected_prefix
            .as_ref()
            .is_some_and(|prefix| name.starts_with(prefix))
        {
            selected_preferred.push(path.clone());
        }
        if package_prefix
            .as_ref()
            .is_some_and(|prefix| name.starts_with(prefix))
        {
            package_preferred.push(path.clone());
        }
        fallback.push(path);
    }
    if !selected_preferred.is_empty() {
        Ok(selected_preferred)
    } else if !package_preferred.is_empty() {
        Ok(package_preferred)
    } else {
        Ok(fallback)
    }
}

fn selected_test_binary_prefix(project_path: &Path, selected_file: &Path) -> Option<String> {
    let args = selected_target_args(project_path, selected_file);
    let target_name = match args.as_slice() {
        [kind] if kind == "--lib" => package_name(project_path)?,
        [kind, name] if matches!(kind.as_str(), "--bin" | "--test" | "--bench") => name.clone(),
        _ => return None,
    };
    Some(target_name.replace('-', "_"))
}

fn coverage_enabled_test_binaries(
    llvm_cov: &Path,
    profdata: &Path,
    binaries: &[PathBuf],
    selected_file: Option<&Path>,
) -> Result<Vec<PathBuf>, String> {
    let mut covered = Vec::new();
    let mut selected_scores = Vec::new();
    let mut failures = Vec::new();
    for binary in binaries {
        let output = Command::new(llvm_cov)
            .arg("export")
            .arg("-summary-only")
            .arg("-format=text")
            .arg("-instr-profile")
            .arg(profdata)
            .arg(binary)
            .output()
            .map_err(|e| format!("failed to run llvm-cov export probe: {e}"))?;
        if output.status.success() {
            if let Some(selected) = selected_file {
                let score = selected_file_covered_score(&output.stdout, selected);
                if score > 0 {
                    selected_scores.push((binary.clone(), score));
                } else {
                    failures.push(format!(
                        "{}: no execution data for {}",
                        binary.display(),
                        selected.display()
                    ));
                }
            } else {
                covered.push(binary.clone());
            }
        } else {
            failures.push(format!(
                "{}: {}",
                binary.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
    }
    if selected_file.is_some() && !selected_scores.is_empty() {
        let best_score = selected_scores
            .iter()
            .map(|(_, score)| *score)
            .max()
            .unwrap_or(0);
        return Ok(selected_scores
            .into_iter()
            .filter_map(|(binary, score)| (score == best_score).then_some(binary))
            .collect());
    }
    if covered.is_empty() {
        Err(format!(
            "no coverage-enabled test binaries found among {} candidate(s): {}",
            binaries.len(),
            failures.join("; ")
        ))
    } else {
        Ok(covered)
    }
}

fn selected_file_covered_score(export_json: &[u8], selected_file: &Path) -> usize {
    let Ok(value) = serde_json::from_slice::<Value>(export_json) else {
        return 0;
    };
    value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|data| {
            data.get("files")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|file_obj| {
            let filename = file_obj.get("filename").and_then(Value::as_str)?;
            let source_file = PathBuf::from(filename);
            same_path_or_suffix(selected_file, &source_file).then_some(file_obj)
        })
        .map(|file_obj| {
            file_obj
                .pointer("/summary/lines/covered")
                .and_then(Value::as_u64)
                .map(|covered| covered as usize)
                .unwrap_or_else(|| selected_file_segment_score(file_obj))
        })
        .sum()
}

fn selected_file_segment_score(file_obj: &Value) -> usize {
    file_obj
        .get("segments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|segment| {
            let Some(array) = segment.as_array() else {
                return false;
            };
            let count = array.get(2).and_then(Value::as_u64).unwrap_or(0);
            let has_count = array.get(3).and_then(Value::as_bool).unwrap_or(false);
            let is_gap = array.get(5).and_then(Value::as_bool).unwrap_or(false);
            count > 0 && has_count && !is_gap
        })
        .count()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.extension().is_none()
}

fn package_name(project_path: &Path) -> Option<String> {
    let manifest = std::fs::read_to_string(project_path.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package {
            if let Some(value) = trimmed.strip_prefix("name") {
                if let Some((_, raw_name)) = value.split_once('=') {
                    let name = raw_name.trim().trim_matches('"');
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
        }
    }
    None
}

fn match_export_file(filename: &str, filter: &file_metrics::ProductionFilter) -> Option<PathBuf> {
    let candidate = PathBuf::from(filename);
    for entry in &filter.files {
        if same_path_or_suffix(&entry.file, &candidate) {
            return Some(entry.file.clone());
        }
    }
    None
}

fn same_path_or_suffix(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if let (Ok(a), Ok(b)) = (a.canonicalize(), b.canonicalize()) {
        if a == b {
            return true;
        }
    }
    let a = normalize_path(a);
    let b = normalize_path(b);
    a.ends_with(&b) || b.ends_with(&a)
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn branch_expression(
    source_file: &Path,
    line_start: usize,
    column_start: usize,
    line_end: usize,
    column_end: usize,
    cache: &mut HashMap<PathBuf, Vec<String>>,
) -> String {
    let lines = cache.entry(source_file.to_path_buf()).or_insert_with(|| {
        std::fs::read_to_string(source_file)
            .map(|source| source.lines().map(str::to_string).collect())
            .unwrap_or_default()
    });
    let Some(line) = lines.get(line_start.saturating_sub(1)) else {
        return String::new();
    };
    if line_start == line_end && column_start > 0 && column_end > column_start {
        let start = column_start.saturating_sub(1);
        let len = column_end.saturating_sub(column_start);
        let slice: String = line.chars().skip(start).take(len).collect();
        let trimmed = slice.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    line.trim().to_string()
}

fn line_numbers(lines: &BTreeSet<(PathBuf, usize)>) -> Vec<usize> {
    let mut numbers: Vec<usize> = lines.iter().map(|(_, line)| *line).collect();
    numbers.sort_unstable();
    numbers.dedup();
    numbers
}

fn percent(part: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 / total as f64 * 100.0
    }
}

fn project_name(project_path: &Path) -> String {
    package_name(project_path).unwrap_or_else(|| {
        project_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    })
}

fn relative_path_string(project_path: &Path, path: &Path) -> String {
    path.strip_prefix(project_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("cstat-coverage-{name}-{unique}"));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        root
    }

    fn write_executable(path: &Path) {
        fs::write(path, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).unwrap();
        }
    }

    #[test]
    fn coverage_target_dir_parses_metadata_target_directory() {
        let root = temp_project("metadata-parse");
        let target = root.join("workspace-target");
        let json = format!(
            r#"{{"target_directory":"{}"}}"#,
            target.to_string_lossy().replace('\\', "\\\\")
        );

        assert_eq!(target_dir_from_metadata_json(&json, &root).unwrap(), target);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn coverage_target_dir_rejects_missing_or_malformed_metadata_target_directory() {
        let root = temp_project("metadata-reject");

        assert!(target_dir_from_metadata_json("{}", &root).is_err());
        assert!(target_dir_from_metadata_json(r#"{"target_directory":42}"#, &root).is_err());
        assert!(target_dir_from_metadata_json("not json", &root).is_err());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn coverage_target_dir_profile_dir_uses_effective_target() {
        let root = temp_project("profile-dir");
        let custom_target = root.with_extension("custom-target");

        assert_eq!(
            coverage_profile_dir(&custom_target),
            custom_target.join("cstat-coverage")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn coverage_target_dir_discovers_binaries_from_effective_target_and_ignores_project_target() {
        let root = temp_project("binary-effective");
        let custom_target = root.with_extension("custom-target");
        let stale_deps = root.join("target/debug/deps");
        let custom_deps = custom_target.join("debug/deps");
        fs::create_dir_all(&stale_deps).unwrap();
        fs::create_dir_all(&custom_deps).unwrap();
        write_executable(&stale_deps.join("fixture-stale"));
        write_executable(&custom_deps.join("fixture-abc"));
        write_executable(&custom_deps.join("other-def"));

        let binaries = discover_test_binaries(&custom_target, &root, None).unwrap();
        let names: Vec<_> = binaries
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert_eq!(names, vec!["fixture-abc"]);
        assert!(!binaries.iter().any(|path| path.ends_with("fixture-stale")));

        fs::remove_dir_all(custom_target).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn coverage_target_dir_falls_back_to_all_executables_without_package_prefix() {
        let root = temp_project("binary-fallback");
        let custom_target = root.with_extension("custom-target");
        let custom_deps = custom_target.join("debug/deps");
        fs::create_dir_all(&custom_deps).unwrap();
        write_executable(&custom_deps.join("alpha"));
        write_executable(&custom_deps.join("beta"));

        let binaries = discover_test_binaries(&custom_target, &root, None).unwrap();
        let names: BTreeSet<_> = binaries
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert_eq!(
            names,
            ["alpha".to_string(), "beta".to_string()]
                .into_iter()
                .collect()
        );

        fs::remove_dir_all(custom_target).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn coverage_target_dir_prefers_selected_binary_target_over_package_prefix() {
        let root = temp_project("binary-selected");
        fs::create_dir_all(root.join("src/bin")).unwrap();
        fs::write(root.join("src/bin/mvp_chat.rs"), "fn main() {}\n").unwrap();
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "mvp-system"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "mvp-chat"
path = "src/bin/mvp_chat.rs"
"#,
        )
        .unwrap();
        let custom_target = root.with_extension("custom-target");
        let custom_deps = custom_target.join("debug/deps");
        fs::create_dir_all(&custom_deps).unwrap();
        write_executable(&custom_deps.join("mvp_system-stale"));
        write_executable(&custom_deps.join("mvp_chat-current"));

        let binaries = discover_test_binaries(
            &custom_target,
            &root,
            Some(&root.join("src/bin/mvp_chat.rs")),
        )
        .unwrap();
        let names: Vec<_> = binaries
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert_eq!(names, vec!["mvp_chat-current"]);

        fs::remove_dir_all(custom_target).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn coverage_target_dir_error_mentions_effective_deps_dir() {
        let root = temp_project("binary-error");
        let custom_target = root.with_extension("custom-target");

        let err = discover_test_binaries(&custom_target, &root, None).unwrap_err();

        assert!(
            err.contains(&custom_target.join("debug/deps").display().to_string()),
            "{err}"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn coverage_rustflags_preserve_existing_flags_and_enable_branch_option() {
        assert_eq!(
            coverage_rustflags(None, false),
            "-C instrument-coverage -C link-dead-code"
        );
        assert_eq!(
            coverage_rustflags(Some("-C debuginfo=2"), false),
            "-C debuginfo=2 -C instrument-coverage -C link-dead-code"
        );
        assert_eq!(
            coverage_rustflags(Some("-C debuginfo=2"), true),
            "-C debuginfo=2 -C instrument-coverage -C link-dead-code -Z coverage-options=branch"
        );
    }

    #[test]
    fn parses_llvm_export_line_and_branch_sites() {
        let root = temp_project("parser");
        let source = root.join("src/lib.rs");
        fs::write(
            &source,
            "pub fn decide(flag: bool) -> u8 {\n    if flag { 1 } else { 0 }\n}\n",
        )
        .unwrap();
        let symbols = ast_parser::parse_project(&[source.clone()]);
        let graph = flow::build_call_graph(&[source.clone()], &root);
        let filter = file_metrics::production_filter_for_files(
            &symbols,
            &graph,
            &root,
            &[source.clone()],
            Some(&source),
        );
        let json = format!(
            r#"{{"type":"llvm.coverage.json.export","data":[{{"files":[{{"filename":"{}","segments":[[1,1,1,true,true,false],[2,5,1,true,true,false],[3,1,0,true,true,false]],"branches":[[2,8,2,12,1,0,0,0,0]],"summary":{{}}}}]}}]}}"#,
            source.to_string_lossy().replace('\\', "\\\\")
        );
        let report = analyze_export_json(&json, &root, Some(&source), &filter).unwrap();
        assert_eq!(report.branch_coverage.sites.len(), 1);
        let site = &report.branch_coverage.sites[0];
        assert_eq!(site.line_start, 2);
        assert_eq!(site.true_count, 1);
        assert_eq!(site.false_count, 0);
        assert_eq!(site.taken, vec!["true"]);
        assert_eq!(site.missing, vec!["false"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selected_coverage_target_infers_lib_bin_and_test_args() {
        let root = temp_project("targets");
        fs::create_dir_all(root.join("src/bin")).unwrap();
        fs::create_dir_all(root.join("tests")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        fs::write(root.join("src/bin/mvp_chat.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("tests/api.rs"), "#[test] fn api() {}\n").unwrap();
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "fixture"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "mvp-chat"
path = "src/bin/mvp_chat.rs"
"#,
        )
        .unwrap();

        let lib_args = cargo_test_args(&root, Some(&root.join("src/lib.rs")));
        assert!(lib_args.ends_with(&["--lib".to_string()]), "{lib_args:?}");
        let bin_args = cargo_test_args(&root, Some(&root.join("src/bin/mvp_chat.rs")));
        assert!(
            bin_args.ends_with(&["--bin".to_string(), "mvp-chat".to_string()]),
            "{bin_args:?}"
        );
        let test_args = cargo_test_args(&root, Some(&root.join("tests/api.rs")));
        assert!(
            test_args.ends_with(&["--test".to_string(), "api".to_string()]),
            "{test_args:?}"
        );

        fs::remove_dir_all(root).unwrap();
    }
}
