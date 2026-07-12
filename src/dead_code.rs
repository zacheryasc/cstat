use colored::Colorize;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::flow::{self, CallGraph};
use crate::render;

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
