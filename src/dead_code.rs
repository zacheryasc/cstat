use colored::Colorize;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::flow::{self, CallGraph};
use crate::{ast_parser, file_metrics, render};

#[derive(Debug, Clone, Serialize)]
pub struct DeadCodeCandidate {
    pub function: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeadCodeReport {
    pub total_functions: usize,
    pub entry_points: Vec<String>,
    pub reachable_count: usize,
    pub candidate_count: usize,
    pub candidates: Vec<DeadCodeCandidate>,
    pub analysis_note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileDeadCodeFunction {
    pub function: String,
    pub reached_from_crate_roots: bool,
    pub crate_fan_in: usize,
    pub in_file_fan_in: usize,
    pub candidate: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileDeadCodeReport {
    pub file: String,
    pub crate_roots: Vec<String>,
    pub file_function_count: usize,
    pub reached_from_crate_count: usize,
    pub candidate_count: usize,
    pub functions: Vec<FileDeadCodeFunction>,
}

pub fn analyze_graph(graph: &CallGraph) -> DeadCodeReport {
    let reachable = flow::reachable_from_entries(graph);
    let mut candidates: Vec<DeadCodeCandidate> = graph
        .functions
        .iter()
        .filter(|function| !reachable.contains(*function))
        .map(|function| DeadCodeCandidate {
            function: function.clone(),
            reason: "not statically reachable from main, tests, or benchmarks".to_string(),
        })
        .collect();
    candidates.sort_by(|a, b| a.function.cmp(&b.function));

    DeadCodeReport {
        total_functions: graph.functions.len(),
        entry_points: graph.entry_points.clone(),
        reachable_count: reachable.len(),
        candidate_count: candidates.len(),
        candidates,
        analysis_note: "Static call graph only; dynamic dispatch, macro-generated calls, and reflection-like string lookups are not resolved.".to_string(),
    }
}

pub fn analyze_files(rs_files: &[PathBuf], project_path: &Path) -> DeadCodeReport {
    let graph = flow::build_call_graph(rs_files, project_path);
    analyze_graph(&graph)
}

pub fn analyze_file_dead_code(
    graph: &CallGraph,
    file_functions: &file_metrics::FileFunctionIndex,
    file_display_path: &str,
) -> FileDeadCodeReport {
    let reachable = flow::reachable_from_entries(graph);
    let selected: HashSet<String> = file_functions
        .functions
        .iter()
        .map(|function| function.graph_name.clone())
        .collect();

    let mut functions = Vec::new();
    for function in file_functions.functions.iter().filter(|function| {
        !function.is_test
            && !function.is_bench
            && !function.is_test_support
            && !function.is_binary_wrapper_main
    }) {
        let mut in_file_fan_in = 0usize;
        let mut crate_fan_in = 0usize;
        for (caller, callees) in &graph.adjacency {
            for callee in callees {
                if callee != &function.graph_name {
                    continue;
                }
                if selected.contains(caller) {
                    in_file_fan_in += 1;
                } else {
                    crate_fan_in += 1;
                }
            }
        }

        let reached_from_crate_roots = reachable.contains(&function.graph_name);
        let candidate = !reached_from_crate_roots;
        let reason = if !candidate {
            "reached from crate roots"
        } else if crate_fan_in == 0 && in_file_fan_in == 0 {
            "not reached from crate roots; no direct, trait, drop, function-item, or closure edge was found"
        } else if crate_fan_in == 0 && in_file_fan_in > 0 {
            "only reached by cold in-file path; no test/benchmark root reaches that caller"
        } else {
            "not reached from crate roots; incoming edge source is cold"
        };

        functions.push(FileDeadCodeFunction {
            function: function.display_name.clone(),
            reached_from_crate_roots,
            crate_fan_in,
            in_file_fan_in,
            candidate,
            reason: reason.to_string(),
        });
    }

    functions.sort_by(|a, b| a.function.cmp(&b.function));
    let reached_from_crate_count = functions
        .iter()
        .filter(|function| function.reached_from_crate_roots)
        .count();
    let candidate_count = functions
        .iter()
        .filter(|function| function.candidate)
        .count();

    FileDeadCodeReport {
        file: file_display_path.to_string(),
        crate_roots: graph.entry_points.clone(),
        file_function_count: functions.len(),
        reached_from_crate_count,
        candidate_count,
        functions,
    }
}

pub fn render_dead_code_report(report: &DeadCodeReport, verbose: bool) {
    render::section_header("Dead code candidates");

    if verbose {
        render::verbose_block(&[
            "Candidates are functions unreachable from known runtime, test, or benchmark roots.",
            "This is conservative static evidence, not compiler-confirmed deletion safety.",
            "Check macro calls, trait dispatch, and public API reachability before deleting.",
        ]);
    }

    println!(
        "  {} {} / {} reachable  {} {} candidates",
        "coverage".cyan(),
        report.reachable_count.to_string().bold(),
        report.total_functions,
        "cold".cyan(),
        report.candidate_count.to_string().bold(),
    );

    if report.entry_points.is_empty() {
        println!(
            "  {}",
            "No entry points found; all functions are cold candidates.".yellow()
        );
    } else {
        println!(
            "  {} {}",
            "entry points:".cyan(),
            report.entry_points.join(", ")
        );
    }

    if report.candidates.is_empty() {
        println!("  {}", "No dead-code candidates found.".green());
        return;
    }

    println!();
    for candidate in &report.candidates {
        println!("  {} {}", "•".yellow(), candidate.function);
    }
}

pub fn render_dead_code(rs_files: &[PathBuf], project_path: &Path, verbose: bool) {
    let report = analyze_files(rs_files, project_path);
    render_dead_code_report(&report, verbose);
}

pub fn render_dead_code_json(rs_files: &[PathBuf], project_path: &Path) {
    let report = analyze_files(rs_files, project_path);
    println!("{}", serde_json::to_string(&report).unwrap());
}

pub fn render_dead_code_file(
    rs_files: &[PathBuf],
    project_path: &Path,
    file: &Path,
    verbose: bool,
) {
    let report = analyze_file_dead_code_from_files(rs_files, project_path, file);
    render_file_dead_code_report(&report, verbose);
}

pub fn render_dead_code_file_json(rs_files: &[PathBuf], project_path: &Path, file: &Path) {
    let report = analyze_file_dead_code_from_files(rs_files, project_path, file);
    println!("{}", serde_json::to_string(&report).unwrap());
}

fn analyze_file_dead_code_from_files(
    rs_files: &[PathBuf],
    project_path: &Path,
    file: &Path,
) -> FileDeadCodeReport {
    let symbols = ast_parser::parse_project(rs_files);
    let graph = flow::build_call_graph(rs_files, project_path);
    let file_functions =
        file_metrics::selected_file_functions(&symbols, &graph, project_path, file);
    let file_display_path = relative_path_string(project_path, file);
    analyze_file_dead_code(&graph, &file_functions, &file_display_path)
}

pub fn render_file_dead_code_report(report: &FileDeadCodeReport, verbose: bool) {
    render::section_header("Dead code candidates");

    if verbose {
        render::verbose_block(&[
            "Candidates are selected-file functions not reachable from crate roots.",
            "Fan-in is split between callers in the selected file and callers elsewhere in the crate.",
            "This is conservative static evidence, not compiler-confirmed deletion safety.",
        ]);
    }

    println!(
        "  {} {} / {}  {} {} candidates",
        "reached from crate roots".cyan(),
        report.reached_from_crate_count.to_string().bold(),
        report.file_function_count,
        "cold".cyan(),
        report.candidate_count.to_string().bold(),
    );
    println!(
        "  {} {}",
        "crate roots:".cyan(),
        report.crate_roots.join(", ")
    );

    println!();
    println!(
        "  {:<48} {:>6} {:>11} {:>13}  {}",
        "function".bold(),
        "cold".bold(),
        "crate fan-in".bold(),
        "in-file fan-in".bold(),
        "reason".bold(),
    );
    if report.functions.is_empty() {
        println!("  {}", "<none>".dimmed());
    } else {
        for function in &report.functions {
            println!(
                "  {:<48} {:>6} {:>11} {:>13}  {}",
                truncate(&function.function, 48),
                if function.candidate { "yes" } else { "no" },
                function.crate_fan_in,
                function.in_file_fan_in,
                function.reason,
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

fn relative_path_string(project_path: &Path, path: &Path) -> String {
    path.strip_prefix(project_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
