use colored::Colorize;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{ast_parser, datapaths, dead_code, deps, flow, loc, render, symbols};

#[derive(Debug, Clone, Serialize)]
pub struct LineCountFile {
    pub path: String,
    pub total_lines: usize,
    pub code_lines: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LineCountReport {
    pub total_files: usize,
    pub total_lines: usize,
    pub total_code_lines: usize,
    pub mean_code_lines: f64,
    pub median_code_lines: usize,
    pub files: Vec<LineCountFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleDegree {
    pub module: String,
    pub fan_in: usize,
    pub fan_out: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleDependencyMatrix {
    pub modules: Vec<String>,
    pub matrix: Vec<Vec<usize>>,
    pub edges: usize,
    pub degrees: Vec<ModuleDegree>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FocusReport {
    pub cstat_version: String,
    pub project: String,
    pub line_counts: LineCountReport,
    pub symbols: symbols::SymbolReport,
    pub dependencies: ModuleDependencyMatrix,
    pub dead_code: dead_code::DeadCodeReport,
    pub test_reachability: datapaths::TestReachabilityAnalysis,
}

pub fn build_report(rs_files: &[PathBuf], project_path: &Path) -> FocusReport {
    let project_path = project_path
        .canonicalize()
        .unwrap_or_else(|_| project_path.to_path_buf());
    let project = project_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let (file_stats, aggregate) = loc::analyze_files(rs_files);
    let symbols_raw = ast_parser::parse_project(rs_files);
    let dep_analysis = deps::analyze_deps(rs_files, &project_path);
    let call_graph = flow::build_call_graph(rs_files, &project_path);

    FocusReport {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        project,
        line_counts: build_line_report(&file_stats, aggregate.as_ref(), &project_path),
        symbols: symbols::analyze_symbols(&symbols_raw, &project_path),
        dependencies: build_dependency_matrix(&dep_analysis),
        dead_code: dead_code::analyze_graph(&call_graph),
        test_reachability: datapaths::analyze_graph_test_reachability(&call_graph),
    }
}

pub fn render_report(rs_files: &[PathBuf], project_path: &Path, verbose: bool) {
    let report = build_report(rs_files, project_path);
    render_report_data(&report, verbose);
}

pub fn render_report_json(rs_files: &[PathBuf], project_path: &Path) {
    let report = build_report(rs_files, project_path);
    println!("{}", serde_json::to_string(&report).unwrap());
}

pub fn render_report_data(report: &FocusReport, verbose: bool) {
    println!(
        "\n{} {}",
        "cstat focused report".bright_cyan().bold(),
        format!("({})", report.project).dimmed()
    );
    println!(
        "{}",
        "Objective module metrics: size, symbols, dependencies, dead-code candidates, and test/bench reachability."
            .dimmed()
    );

    render_line_counts(&report.line_counts, verbose);
    symbols::render_symbol_report(&report.symbols, verbose);
    render_dependency_matrix(&report.dependencies, verbose);
    dead_code::render_dead_code_report(&report.dead_code, verbose);
    datapaths::render_test_reachability_report(&report.test_reachability, verbose);
}

fn build_line_report(
    file_stats: &[loc::FileLocStats],
    aggregate: Option<&loc::AggregateStats>,
    project_path: &Path,
) -> LineCountReport {
    let total_lines = file_stats.iter().map(|file| file.total_lines).sum();
    let files: Vec<LineCountFile> = file_stats
        .iter()
        .map(|file| LineCountFile {
            path: file
                .path
                .strip_prefix(project_path)
                .unwrap_or(&file.path)
                .display()
                .to_string(),
            total_lines: file.total_lines,
            code_lines: file.code_lines,
        })
        .collect();

    LineCountReport {
        total_files: aggregate.map_or(file_stats.len(), |agg| agg.total_files),
        total_lines,
        total_code_lines: aggregate.map_or(0, |agg| agg.total_loc),
        mean_code_lines: aggregate.map_or(0.0, |agg| agg.mean),
        median_code_lines: aggregate.map_or(0, |agg| agg.median),
        files,
    }
}

fn build_dependency_matrix(dep_analysis: &deps::DepAnalysis) -> ModuleDependencyMatrix {
    let modules = dep_analysis.modules.clone();
    let module_index: HashMap<&str, usize> = modules
        .iter()
        .enumerate()
        .map(|(idx, module)| (module.as_str(), idx))
        .collect();
    let mut matrix = vec![vec![0usize; modules.len()]; modules.len()];

    for edge in &dep_analysis.edges {
        if let (Some(from), Some(to)) = (
            module_index.get(edge.from.as_str()),
            module_index.get(edge.to.as_str()),
        ) {
            matrix[*from][*to] = 1;
        }
    }

    let mut degrees: Vec<ModuleDegree> = modules
        .iter()
        .map(|module| ModuleDegree {
            module: module.clone(),
            fan_in: dep_analysis.in_degree.get(module).copied().unwrap_or(0),
            fan_out: dep_analysis.out_degree.get(module).copied().unwrap_or(0),
        })
        .collect();
    degrees.sort_by(|a, b| {
        (b.fan_in + b.fan_out)
            .cmp(&(a.fan_in + a.fan_out))
            .then(a.module.cmp(&b.module))
    });

    ModuleDependencyMatrix {
        modules,
        matrix,
        edges: dep_analysis.edges.len(),
        degrees,
    }
}

fn render_line_counts(report: &LineCountReport, verbose: bool) {
    render::section_header("Line counts");

    if verbose {
        render::verbose_block(&[
            "Code lines exclude blank lines and comment-only lines.",
            "Physical lines are kept beside code lines so generated or dense files are visible.",
        ]);
    }

    println!(
        "  {} {}  {} {}  {} {:.1}  {} {}",
        "files".cyan(),
        report.total_files.to_string().bold(),
        "code lines".cyan(),
        report.total_code_lines.to_string().bold(),
        "mean".cyan(),
        report.mean_code_lines,
        "median".cyan(),
        report.median_code_lines.to_string().bold(),
    );

    println!();
    println!(
        "  {:<56} {:>8} {:>8}",
        "file".bold(),
        "code".bold(),
        "physical".bold()
    );
    for file in &report.files {
        println!(
            "  {:<56} {:>8} {:>8}",
            truncate(&file.path, 56),
            file.code_lines,
            file.total_lines,
        );
    }
}

fn render_dependency_matrix(report: &ModuleDependencyMatrix, verbose: bool) {
    render::section_header("Module dependency matrix");

    if verbose {
        render::verbose_block(&[
            "Rows are modules that depend on columns.",
            "A 1 means at least one source-level use/mod edge from row to column.",
            "The index list keeps the matrix readable for medium-sized crates.",
        ]);
    }

    println!(
        "  {} {}  {} {}",
        "modules".cyan(),
        report.modules.len().to_string().bold(),
        "edges".cyan(),
        report.edges.to_string().bold(),
    );

    if report.modules.is_empty() {
        println!("  {}", "No modules found.".yellow());
        return;
    }

    println!();
    for (idx, module) in report.modules.iter().enumerate() {
        println!("  [{:>2}] {}", idx, module);
    }

    let cell_width = report
        .modules
        .len()
        .saturating_sub(1)
        .to_string()
        .len()
        .max(1);
    println!();
    print!("  {:>3} │", "");
    for idx in 0..report.modules.len() {
        print!(" {:>width$}", idx, width = cell_width);
    }
    println!();
    print!("  {}─┼", "─".repeat(3));
    for _ in 0..report.modules.len() {
        print!("{}", "─".repeat(cell_width + 1));
    }
    println!();

    for (idx, row) in report.matrix.iter().enumerate() {
        print!("  {:>3} │", idx);
        for value in row {
            print!(" {:>width$}", value, width = cell_width);
        }
        println!();
    }

    let connected: Vec<&ModuleDegree> = report
        .degrees
        .iter()
        .filter(|degree| degree.fan_in + degree.fan_out > 0)
        .take(10)
        .collect();
    if !connected.is_empty() {
        println!();
        println!("  {}", "Highest fan-in/fan-out:".bold());
        for degree in connected {
            println!(
                "  {:<40} in {:>3}  out {:>3}",
                truncate(&degree.module, 40),
                degree.fan_in,
                degree.fan_out,
            );
        }
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
