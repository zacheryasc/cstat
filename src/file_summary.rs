use colored::Colorize;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::{
    ast_parser, call_trace, datapaths, dead_code, deps, file_metrics, flow, loc, render, symbols,
};

#[derive(Debug, Clone, Serialize)]
pub struct FileSummaryReport {
    pub cstat_version: String,
    pub file: String,
    pub project: String,
    pub line_counts: loc::FileProjectedLineReachabilityReport,
    pub static_reachability: datapaths::FileStaticReachabilityReport,
    pub symbols: symbols::FileSymbolReport,
    pub dependencies: deps::FileDependencyReport,
    pub dead_code: dead_code::FileDeadCodeReport,
    pub call_trace: call_trace::FileCallTraceReport,
}

pub fn render_file_summary(rs_files: &[PathBuf], project_path: &Path, file: &Path, verbose: bool) {
    let report = build_file_summary(rs_files, project_path, file);
    render_file_summary_report(&report, verbose);
}

pub fn render_file_summary_json(rs_files: &[PathBuf], project_path: &Path, file: &Path) {
    let report = build_file_summary(rs_files, project_path, file);
    println!("{}", serde_json::to_string(&report).unwrap());
}

pub fn build_file_summary(
    rs_files: &[PathBuf],
    project_path: &Path,
    file: &Path,
) -> FileSummaryReport {
    let file_display_path = call_trace::relative_path_string(project_path, file);
    let project = project_name(project_path);

    let symbols_raw = ast_parser::parse_project(rs_files);
    let graph = flow::build_call_graph(rs_files, project_path);
    let file_functions =
        file_metrics::selected_file_functions(&symbols_raw, &graph, project_path, file);
    let static_reachability =
        datapaths::analyze_file_static_reachability(&graph, &file_functions, &file_display_path);
    let reachable_functions: HashSet<String> = static_reachability
        .reachable_functions
        .iter()
        .cloned()
        .collect();
    let line_counts = loc::analyze_file_projected_line_reachability(
        file,
        &file_functions,
        &reachable_functions,
        &file_display_path,
    )
    .unwrap_or_else(|| empty_line_report(file, &file_display_path));
    let symbols = symbols::analyze_symbols_file(&symbols_raw, project_path, file);
    let dependencies = deps::analyze_file_deps(rs_files, project_path, file);
    let dead_code = dead_code::analyze_file_dead_code(&graph, &file_functions, &file_display_path);
    let call_trace =
        call_trace::analyze_file_call_traces(&graph, &file_functions, &file_display_path);

    FileSummaryReport {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        file: file_display_path,
        project,
        line_counts,
        static_reachability,
        symbols,
        dependencies,
        dead_code,
        call_trace,
    }
}

fn project_name(project_path: &Path) -> String {
    let fallback = || {
        project_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    };

    let Ok(manifest) = std::fs::read_to_string(project_path.join("Cargo.toml")) else {
        return fallback();
    };

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
                        return name.to_string();
                    }
                }
            }
        }
    }

    fallback()
}

fn empty_line_report(
    file: &Path,
    file_display_path: &str,
) -> loc::FileProjectedLineReachabilityReport {
    let stats = loc::count_file_lines(file);
    loc::FileProjectedLineReachabilityReport {
        file: file_display_path.to_string(),
        total_lines: stats.as_ref().map_or(0, |stats| stats.total_lines),
        code_lines: 0,
        projected_reachable_lines: 0,
        projected_unreachable_lines: 0,
        reachable_spans: Vec::new(),
        unreachable_spans: Vec::new(),
    }
}

fn render_file_summary_report(report: &FileSummaryReport, verbose: bool) {
    println!(
        "\n{} {}",
        "cstat file summary".bright_cyan().bold(),
        format!("({})", report.file).dimmed(),
    );
    println!(
        "{}",
        "Single-file static metrics projected through the containing crate.".dimmed()
    );

    render_line_counts(&report.line_counts, &report.file, verbose);
    render_static_reachability(&report.static_reachability, &report.file, verbose);
    render_symbols(&report.symbols, &report.file, verbose);
    render_dependencies(&report.dependencies, &report.file, verbose);
    render_dead_code(&report.dead_code, &report.file, verbose);
    render_call_trace(&report.call_trace, &report.file, verbose);
}

fn render_line_counts(
    report: &loc::FileProjectedLineReachabilityReport,
    file: &str,
    verbose: bool,
) {
    render::section_header("Projected line reachability");
    if verbose {
        let full_section = format!("Full section: cstat loc --path {file} -v");
        render::verbose_block(&[
            "Production code lines exclude blank/comment-only lines and test/support/wrapper spans.",
            "Projected reachable lines are static function spans reached from project tests/benches.",
            full_section.as_str(),
        ]);
    }

    let pct = percentage(report.projected_reachable_lines, report.code_lines);
    println!(
        "  {} {}  {} {}  {} {} / {} ({:.1}%)  {} {}",
        "production code lines".cyan(),
        report.code_lines.to_string().bold(),
        "physical".cyan(),
        report.total_lines.to_string().bold(),
        "statically reachable".cyan(),
        report.projected_reachable_lines.to_string().bold(),
        report.code_lines,
        pct,
        "not statically reachable".cyan(),
        report.projected_unreachable_lines.to_string().bold(),
    );

    if !report.reachable_spans.is_empty() {
        println!();
        println!("  {}", "reachable spans:".bold());
        for span in report.reachable_spans.iter().take(6) {
            println!(
                "  {} lines {}-{}",
                span.function, span.line_start, span.line_end
            );
        }
        print_more(report.reachable_spans.len(), 6);
    }

    if !report.unreachable_spans.is_empty() {
        println!();
        println!("  {}", "unreachable spans:".bold().yellow());
        for span in report.unreachable_spans.iter().take(6) {
            println!(
                "  {} lines {}-{}",
                span.function, span.line_start, span.line_end
            );
        }
        print_more(report.unreachable_spans.len(), 6);
    }
}

fn render_static_reachability(
    report: &datapaths::FileStaticReachabilityReport,
    file: &str,
    verbose: bool,
) {
    render::section_header("Static test/benchmark reachability");

    if verbose {
        let full_section = format!("Full section: cstat test-reachability --path {file} -v");
        render::verbose_block(&[
            "Roots are tests and benchmarks discovered anywhere in the project.",
            "Reachability follows the full crate call graph, then projects production results onto this file.",
            full_section.as_str(),
        ]);
    }

    let pct = percentage(
        report.reachable_function_count,
        report.production_function_count,
    );
    println!(
        "  {} {}  {} {} / {} ({:.1}%)  {} {} / {}",
        "test/bench roots".cyan(),
        report.test_benchmark_entry_count.to_string().bold(),
        "functions reachable".cyan(),
        report.reachable_function_count.to_string().bold(),
        report.production_function_count,
        pct,
        "local edges reached".cyan(),
        report.reachable_local_edge_count.to_string().bold(),
        report.local_edge_count,
    );

    if !report.wrapper_entrypoints.is_empty() {
        println!(
            "  {} {}",
            "wrapper entrypoints:".cyan(),
            report.wrapper_entrypoints.join(", ")
        );
    }

    if !report.entries.is_empty() {
        println!();
        println!(
            "  {:<48} {:<7} {:>9} {:>7} {:>8}",
            "entry root".bold(),
            "kind".bold(),
            "functions".bold(),
            "steps".bold(),
            "depth".bold(),
        );
        for entry in report.entries.iter().take(8) {
            println!(
                "  {:<48} {:<7} {:>9} {:>7} {:>8}",
                truncate(&entry.entry, 48),
                entry.kind,
                entry.reachable_functions,
                entry.unique_call_steps,
                entry.max_call_depth,
            );
        }
        print_more(report.entries.len(), 8);
    }

    println!();
    println!("  {}", "not statically reachable:".bold().yellow());
    if report.unreachable_functions.is_empty() {
        println!("  {}", "<none>".dimmed());
    } else {
        for function in report.unreachable_functions.iter().take(8) {
            println!("  {} {}", "•".yellow(), function);
        }
        print_more(report.unreachable_functions.len(), 8);
    }
}

fn render_symbols(report: &symbols::FileSymbolReport, file: &str, verbose: bool) {
    render::section_header("Symbols");
    if verbose {
        let full_section = format!("Full section: cstat symbols --path {file} -v");
        render::verbose_block(&[
            "Selected-file symbols use syn Rust AST item discovery.",
            "Functions include free functions and impl methods once.",
            full_section.as_str(),
        ]);
    }

    println!(
        "  {} {}  {} {}  {} {}  {} {}  {} {}  {} {}  {} {}  {} {}",
        "total".cyan(),
        report.total.to_string().bold(),
        "fn".cyan(),
        report.functions.to_string().bold(),
        "struct".cyan(),
        report.structs.to_string().bold(),
        "enum".cyan(),
        report.enums.to_string().bold(),
        "trait".cyan(),
        report.traits.to_string().bold(),
        "trait_impl".cyan(),
        report.trait_impls.to_string().bold(),
        "const".cyan(),
        report.consts.to_string().bold(),
        "static".cyan(),
        report.statics.to_string().bold(),
    );

    if report.parse_error {
        println!(
            "  {}",
            "parse error: selected file could not be parsed"
                .yellow()
                .bold()
        );
    }

    if !report.symbols.is_empty() {
        println!();
        println!("  {:<10} {}", "kind".bold(), "symbol".bold());
        for row in report.symbols.iter().take(12) {
            println!(
                "  {:<10} {}",
                file_symbol_kind_label(&row.kind),
                truncate(&row.symbol, 60),
            );
        }
        print_more(report.symbols.len(), 12);
    }
}

fn render_dependencies(report: &deps::FileDependencyReport, file: &str, verbose: bool) {
    render::section_header("Dependencies");
    if verbose {
        let full_section = format!("Full section: cstat deps --path {file} -v");
        render::verbose_block(&[
            "Imports are raw use/mod targets parsed from this file.",
            "Internal imports resolve to known modules in the containing crate.",
            full_section.as_str(),
        ]);
    }

    println!(
        "  {} {}  {} {}  {} {}  {} {}",
        "module".cyan(),
        report.module.bold(),
        "imports".cyan(),
        report.imports.to_string().bold(),
        "internal".cyan(),
        report.internal.len().to_string().bold(),
        "external".cyan(),
        report.external.len().to_string().bold(),
    );

    print_dependency_list("internal", &report.internal, 6, true);
    print_dependency_list("external", &report.external, 6, false);
}

fn render_dead_code(report: &dead_code::FileDeadCodeReport, file: &str, verbose: bool) {
    render::section_header("Dead code candidates");
    if verbose {
        let full_section = format!("Full section: cstat dead-code --path {file} -v");
        render::verbose_block(&[
            "Candidates are file functions not reachable from crate roots.",
            "Fan-in distinguishes crate callers from callers inside this file.",
            full_section.as_str(),
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

    let candidates: Vec<_> = report
        .functions
        .iter()
        .filter(|function| function.candidate)
        .collect();
    println!();
    println!(
        "  {:<44} {:>11} {:>13}  {}",
        "function".bold(),
        "crate fan-in".bold(),
        "in-file fan-in".bold(),
        "reason".bold(),
    );
    if candidates.is_empty() {
        println!("  {}", "<none>".dimmed());
    } else {
        for function in candidates.iter().take(8) {
            println!(
                "  {:<44} {:>11} {:>13}  {}",
                truncate(&function.function, 44),
                function.crate_fan_in,
                function.in_file_fan_in,
                function.reason,
            );
        }
        print_more(candidates.len(), 8);
    }
}

fn render_call_trace(report: &call_trace::FileCallTraceReport, file: &str, verbose: bool) {
    render::section_header("Call trace size");
    if verbose {
        let full_section = format!("Full section: cstat call-trace --path {file} -v");
        render::verbose_block(&[
            "Rows rank non-test functions by the size of their static call tree.",
            "Direct callees are truncated for the summary; full trees are in the section command.",
            full_section.as_str(),
        ]);
    }

    println!(
        "  {:<44} {:>7} {:>5}  {}",
        "function".bold(),
        "reached".bold(),
        "depth".bold(),
        "direct callees".bold(),
    );
    let width = render::terminal_width();
    let callee_width = width.saturating_sub(64).max(24);
    for function in report.functions.iter().take(12) {
        let callees = if function.direct_callees.is_empty() {
            "-".to_string()
        } else {
            function.direct_callees.join(", ")
        };
        println!(
            "  {:<44} {:>7} {:>5}  {}",
            truncate(&function.function, 44),
            function.functions_reached,
            function.max_depth,
            truncate(&callees, callee_width),
        );
    }
    print_more(report.functions.len(), 12);
}

fn print_dependency_list(
    label: &str,
    dependencies: &[deps::FileDependency],
    limit: usize,
    show_resolved: bool,
) {
    println!();
    println!("  {}", format!("{label} imports:").bold());
    if dependencies.is_empty() {
        println!("  {}", "<none>".dimmed());
        return;
    }

    for dependency in dependencies.iter().take(limit) {
        if show_resolved {
            println!(
                "  {} {} -> {}",
                "•".cyan(),
                dependency.raw,
                dependency.resolved_module.as_deref().unwrap_or("-"),
            );
        } else {
            println!("  {} {}", "•".cyan(), dependency.raw);
        }
    }
    print_more(dependencies.len(), limit);
}

fn file_symbol_kind_label(kind: &symbols::FileSymbolKind) -> &'static str {
    match kind {
        symbols::FileSymbolKind::Fn => "fn",
        symbols::FileSymbolKind::Struct => "struct",
        symbols::FileSymbolKind::Enum => "enum",
        symbols::FileSymbolKind::Trait => "trait",
        symbols::FileSymbolKind::TraitImpl => "trait_impl",
        symbols::FileSymbolKind::Const => "const",
        symbols::FileSymbolKind::Static => "static",
    }
}

fn percentage(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64 * 100.0
    }
}

fn print_more(total: usize, shown: usize) {
    if total > shown {
        println!("  … +{} more", total - shown);
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
