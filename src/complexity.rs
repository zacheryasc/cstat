use colored::Colorize;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::ast_parser::{FunctionInfo, ProjectSymbols};
use crate::render;

#[derive(Debug, Clone, Serialize)]
pub struct BranchingRow {
    pub function: String,
    pub file: String,
    pub cyclomatic: usize,
    pub branch_points: usize,
    pub nesting_depth: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignatureRow {
    pub function: String,
    pub file: String,
    pub param_count: usize,
    pub return_type_complexity: usize,
    pub generic_param_count: usize,
    pub trait_bound_count: usize,
    pub where_predicate_count: usize,
    pub signature_score: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignatureFileRow {
    pub file: String,
    pub trait_impl_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpanRow {
    pub function: String,
    pub file: String,
    pub line_count: usize,
    pub body_stmt_count: usize,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Serialize)]
struct BranchingJson {
    cstat_version: String,
    functions: Vec<BranchingRow>,
}

#[derive(Serialize)]
struct SignatureJson {
    cstat_version: String,
    functions: Vec<SignatureRow>,
    files: Vec<SignatureFileRow>,
}

#[derive(Serialize)]
struct SpanJson {
    cstat_version: String,
    functions: Vec<SpanRow>,
}

fn strip_project_path(path: &str, project_path: &Path) -> String {
    let prefix = project_path.display().to_string();
    let stripped = path.strip_prefix(&prefix).unwrap_or(path);
    let stripped = stripped.strip_prefix('/').unwrap_or(stripped);
    stripped.to_string()
}

fn relative_file(path: &Path, project_path: &Path) -> String {
    strip_project_path(&path.display().to_string(), project_path)
}

fn function_line_count(func: &FunctionInfo) -> usize {
    if func.line_end >= func.line_start {
        func.line_end - func.line_start + 1
    } else {
        func.body_stmt_count.max(1)
    }
}

fn signature_score(func: &FunctionInfo) -> usize {
    func.param_count
        + func.return_type_complexity
        + func.generic_param_count
        + func.trait_bound_count
        + func.where_predicate_count
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }

    let keep = max.saturating_sub(3);
    let mut out = value.chars().take(keep).collect::<String>();
    out.push_str("...");
    out
}

pub fn collect_branching(symbols: &ProjectSymbols, project_path: &Path) -> Vec<BranchingRow> {
    let mut rows: Vec<BranchingRow> = symbols
        .files
        .iter()
        .flat_map(|file| file.functions.iter())
        .map(|func| BranchingRow {
            function: func.name.clone(),
            file: relative_file(&func.file, project_path),
            cyclomatic: func.branch_points + 1,
            branch_points: func.branch_points,
            nesting_depth: func.nesting_depth,
        })
        .collect();

    rows.sort_by(|a, b| {
        b.cyclomatic
            .cmp(&a.cyclomatic)
            .then_with(|| b.branch_points.cmp(&a.branch_points))
            .then_with(|| b.nesting_depth.cmp(&a.nesting_depth))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.function.cmp(&b.function))
    });
    rows
}

pub fn collect_signatures(symbols: &ProjectSymbols, project_path: &Path) -> Vec<SignatureRow> {
    let mut rows: Vec<SignatureRow> = symbols
        .files
        .iter()
        .flat_map(|file| file.functions.iter())
        .map(|func| SignatureRow {
            function: func.name.clone(),
            file: relative_file(&func.file, project_path),
            param_count: func.param_count,
            return_type_complexity: func.return_type_complexity,
            generic_param_count: func.generic_param_count,
            trait_bound_count: func.trait_bound_count,
            where_predicate_count: func.where_predicate_count,
            signature_score: signature_score(func),
        })
        .collect();

    rows.sort_by(|a, b| {
        b.signature_score
            .cmp(&a.signature_score)
            .then_with(|| b.param_count.cmp(&a.param_count))
            .then_with(|| b.return_type_complexity.cmp(&a.return_type_complexity))
            .then_with(|| b.generic_param_count.cmp(&a.generic_param_count))
            .then_with(|| b.trait_bound_count.cmp(&a.trait_bound_count))
            .then_with(|| b.where_predicate_count.cmp(&a.where_predicate_count))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.function.cmp(&b.function))
    });
    rows
}

pub fn collect_signature_files(
    symbols: &ProjectSymbols,
    project_path: &Path,
) -> Vec<SignatureFileRow> {
    let mut rows: Vec<SignatureFileRow> = symbols
        .files
        .iter()
        .map(|file_syms| SignatureFileRow {
            file: relative_file(&file_syms.path, project_path),
            trait_impl_count: file_syms
                .impls
                .iter()
                .filter(|imp| imp.trait_name.is_some())
                .count(),
        })
        .collect();

    rows.sort_by(|a, b| {
        b.trait_impl_count
            .cmp(&a.trait_impl_count)
            .then_with(|| a.file.cmp(&b.file))
    });
    rows
}

pub fn collect_spans(symbols: &ProjectSymbols, project_path: &Path) -> Vec<SpanRow> {
    let mut rows: Vec<SpanRow> = symbols
        .files
        .iter()
        .flat_map(|file| file.functions.iter())
        .map(|func| SpanRow {
            function: func.name.clone(),
            file: relative_file(&func.file, project_path),
            line_count: function_line_count(func),
            body_stmt_count: func.body_stmt_count,
            line_start: func.line_start,
            line_end: func.line_end,
        })
        .collect();

    rows.sort_by(|a, b| {
        b.line_count
            .cmp(&a.line_count)
            .then_with(|| b.body_stmt_count.cmp(&a.body_stmt_count))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.function.cmp(&b.function))
    });
    rows
}

pub fn render_branching(symbols: &ProjectSymbols, project_path: &Path, verbose: bool) {
    let rows = collect_branching(symbols, project_path);
    if rows.is_empty() {
        println!("{}", "No functions found to analyze.".yellow());
        return;
    }

    render::section_header("Function Branching Complexity");
    if verbose {
        render::verbose_block(&[
            "Functions ranked by decision/path complexity.",
            "Cyclomatic complexity is branch_points + 1.",
            "Branch points and nesting depth come from the parsed control-flow visitor.",
            "cognitive and composite_score are intentionally not part of this view.",
        ]);
    }

    let display_count = rows.len().min(20);
    println!(
        "{} functions analyzed, showing top {}\n",
        format!("{}", rows.len()).bold(),
        format!("{}", display_count).bold(),
    );
    println!(
        "  {:<40} {:<30} {:>5} {:>8} {:>5}",
        "Function".bold().underline(),
        "File".bold().underline(),
        "Cycl".bold().underline(),
        "Branches".bold().underline(),
        "Nest".bold().underline(),
    );

    for row in rows.iter().take(display_count) {
        println!(
            "  {:<40} {:<30} {:>5} {:>8} {:>5}",
            truncate(&row.function, 40).dimmed(),
            truncate(&row.file, 30).dimmed(),
            row.cyclomatic,
            row.branch_points,
            row.nesting_depth,
        );
    }

    if rows.len() > display_count {
        println!(
            "{}",
            format!("  ... and {} more functions", rows.len() - display_count).dimmed()
        );
    }
    println!();
}

pub fn render_branching_json(symbols: &ProjectSymbols, project_path: &Path) {
    let output = BranchingJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        functions: collect_branching(symbols, project_path),
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}

pub fn render_signature(symbols: &ProjectSymbols, project_path: &Path, verbose: bool) {
    let rows = collect_signatures(symbols, project_path);
    let file_rows = collect_signature_files(symbols, project_path);

    render::section_header("Function Signature Complexity");
    if verbose {
        render::verbose_block(&[
            "Functions ranked by API-boundary complexity.",
            "Signature score = params + return type complexity + generic params + trait bounds + where predicates.",
            "This view answers call/satisfy/reason-about difficulty, not control-flow or implementation span.",
        ]);
    }

    if rows.is_empty() {
        println!("{}", "No functions found to analyze.".yellow());
    } else {
        let display_count = rows.len().min(20);
        println!(
            "{} functions analyzed, showing top {}\n",
            format!("{}", rows.len()).bold(),
            format!("{}", display_count).bold(),
        );
        println!(
            "  {:<34} {:<24} {:>6} {:>6} {:>8} {:>6} {:>5} {:>5}",
            "Function".bold().underline(),
            "File".bold().underline(),
            "Params".bold().underline(),
            "Return".bold().underline(),
            "Generics".bold().underline(),
            "Bounds".bold().underline(),
            "Where".bold().underline(),
            "Score".bold().underline(),
        );

        for row in rows.iter().take(display_count) {
            println!(
                "  {:<34} {:<24} {:>6} {:>6} {:>8} {:>6} {:>5} {:>5}",
                truncate(&row.function, 34).dimmed(),
                truncate(&row.file, 24).dimmed(),
                row.param_count,
                row.return_type_complexity,
                row.generic_param_count,
                row.trait_bound_count,
                row.where_predicate_count,
                row.signature_score,
            );
        }

        if rows.len() > display_count {
            println!(
                "{}",
                format!("  ... and {} more functions", rows.len() - display_count).dimmed()
            );
        }
    }

    if !file_rows.is_empty() {
        render::section_header("Trait Implementations by File");
        let display_count = file_rows.len().min(20);
        println!(
            "  {:<40} {:>10}",
            "File".bold().underline(),
            "TraitImpls".bold().underline(),
        );
        for row in file_rows.iter().take(display_count) {
            println!(
                "  {:<40} {:>10}",
                truncate(&row.file, 40).dimmed(),
                row.trait_impl_count,
            );
        }
        if file_rows.len() > display_count {
            println!(
                "{}",
                format!("  ... and {} more files", file_rows.len() - display_count).dimmed()
            );
        }
    }
    println!();
}

pub fn render_signature_json(symbols: &ProjectSymbols, project_path: &Path) {
    let output = SignatureJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        functions: collect_signatures(symbols, project_path),
        files: collect_signature_files(symbols, project_path),
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}

pub fn render_span(symbols: &ProjectSymbols, project_path: &Path, verbose: bool) {
    let rows = collect_spans(symbols, project_path);
    if rows.is_empty() {
        println!("{}", "No functions found to analyze.".yellow());
        return;
    }

    render::section_header("Function Implementation Span");
    if verbose {
        render::verbose_block(&[
            "Functions ranked by implementation-unit size.",
            "line_count is the inclusive function-local line_start..=line_end span.",
            "This does not replace cstat loc, which remains file/project LOC.",
        ]);
    }

    let display_count = rows.len().min(20);
    println!(
        "{} functions analyzed, showing top {}\n",
        format!("{}", rows.len()).bold(),
        format!("{}", display_count).bold(),
    );
    println!(
        "  {:<40} {:<30} {:>5} {:>5} {:>5} {:>5}",
        "Function".bold().underline(),
        "File".bold().underline(),
        "Lines".bold().underline(),
        "Stmts".bold().underline(),
        "Start".bold().underline(),
        "End".bold().underline(),
    );

    for row in rows.iter().take(display_count) {
        println!(
            "  {:<40} {:<30} {:>5} {:>5} {:>5} {:>5}",
            truncate(&row.function, 40).dimmed(),
            truncate(&row.file, 30).dimmed(),
            row.line_count,
            row.body_stmt_count,
            row.line_start,
            row.line_end,
        );
    }

    if rows.len() > display_count {
        println!(
            "{}",
            format!("  ... and {} more functions", rows.len() - display_count).dimmed()
        );
    }
    println!();
}

pub fn render_span_json(symbols: &ProjectSymbols, project_path: &Path) {
    let output = SpanJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        functions: collect_spans(symbols, project_path),
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}

pub fn run_branching(rs_files: &[PathBuf], project_path: &Path, json: bool, verbose: bool) {
    let symbols = crate::ast_parser::parse_project(rs_files);
    if json {
        render_branching_json(&symbols, project_path);
    } else {
        render_branching(&symbols, project_path, verbose);
    }
}

pub fn run_signature(rs_files: &[PathBuf], project_path: &Path, json: bool, verbose: bool) {
    let symbols = crate::ast_parser::parse_project(rs_files);
    if json {
        render_signature_json(&symbols, project_path);
    } else {
        render_signature(&symbols, project_path, verbose);
    }
}

pub fn run_span(rs_files: &[PathBuf], project_path: &Path, json: bool, verbose: bool) {
    let symbols = crate::ast_parser::parse_project(rs_files);
    if json {
        render_span_json(&symbols, project_path);
    } else {
        render_span(&symbols, project_path, verbose);
    }
}
