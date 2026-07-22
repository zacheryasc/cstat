use colored::Colorize;
use serde::Serialize;
use std::path::{Path, PathBuf};
use syn::visit::Visit;
use syn::Expr;

use crate::ast_parser::{FunctionInfo, ProjectSymbols};
use crate::render;

/// Complexity metrics for a single function.
#[derive(Debug, Clone)]
pub struct FunctionComplexity {
    pub name: String,
    pub file: String,
    pub cyclomatic: usize,
    pub cognitive: usize,
    pub nesting_depth: usize,
    pub composite_score: f64,
    pub line_count: usize,
    pub param_count: usize,
}

/// Aggregate stats across all functions.
pub struct ComplexityStats {
    pub total_functions: usize,
    pub mean: f64,
    pub std_dev: f64,
    pub median: f64,
    pub max: f64,
}

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

/// Compute complexity metrics for a single function.
fn compute_function_complexity(func: &FunctionInfo, source: &str) -> FunctionComplexity {
    // Cyclomatic = branch_points + 1
    let cyclomatic = func.branch_points + 1;

    // Cognitive complexity: re-parse the function body for weighted nesting penalties
    let cognitive = compute_cognitive_complexity(func, source);

    // Generic complexity: type params + trait bounds + where predicates
    let generic_complexity =
        func.generic_param_count + func.trait_bound_count + func.where_predicate_count;

    // Parameter complexity: params weighted by return type complexity
    let param_complexity = func.param_count + func.return_type_complexity;

    // Composite score: weighted combination
    let composite_score = cyclomatic as f64 * 1.0
        + cognitive as f64 * 0.5
        + func.nesting_depth as f64 * 0.3
        + generic_complexity as f64 * 0.2
        + param_complexity as f64 * 0.1;

    let line_count = function_line_count(func);

    FunctionComplexity {
        name: func.name.clone(),
        file: func.file.display().to_string(),
        cyclomatic,
        cognitive,
        nesting_depth: func.nesting_depth,
        composite_score,
        line_count,
        param_count: func.param_count,
    }
}

/// Compute cognitive complexity for a function.
/// Increments for each control flow break, with extra penalties for nesting.
/// Based on SonarSource's cognitive complexity model.
fn compute_cognitive_complexity(func: &FunctionInfo, source: &str) -> usize {
    // Try to find and re-parse the function to walk its body with cognitive scoring
    // We use the already-extracted data as a fallback
    if let Some(cog) = try_cognitive_from_source(func, source) {
        return cog;
    }
    // Fallback: estimate from extracted data
    // branch_points + nesting_depth penalty
    func.branch_points + func.nesting_depth
}

/// Try to compute cognitive complexity by re-parsing the source file and finding the function.
fn try_cognitive_from_source(func: &FunctionInfo, source: &str) -> Option<usize> {
    let ast = syn::parse_file(source).ok()?;
    let mut finder = CognitiveFinder {
        target_name: func.name.clone(),
        result: None,
    };
    finder.walk_items(&ast.items);
    finder.result
}

struct CognitiveFinder {
    target_name: String,
    result: Option<usize>,
}

impl CognitiveFinder {
    fn walk_items(&mut self, items: &[syn::Item]) {
        for item in items {
            match item {
                syn::Item::Fn(f) => {
                    if f.sig.ident == self.target_name {
                        let mut visitor = CognitiveVisitor::new();
                        visitor.visit_block(&f.block);
                        self.result = Some(visitor.complexity);
                    }
                }
                syn::Item::Impl(imp) => {
                    for impl_item in &imp.items {
                        if let syn::ImplItem::Fn(method) = impl_item {
                            let target_type = crate::ast_parser::type_to_string_pub(&imp.self_ty);
                            let qualified = format!("{}::{}", target_type, method.sig.ident);
                            if qualified == self.target_name {
                                let mut visitor = CognitiveVisitor::new();
                                visitor.visit_block(&method.block);
                                self.result = Some(visitor.complexity);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Visitor that computes cognitive complexity using SonarSource-style rules:
/// - +1 for each control flow break (if, else if, else, match, for, while, loop, break, continue, &&, ||)
/// - +1 nesting increment for each level of nesting when encountering a break
struct CognitiveVisitor {
    nesting: usize,
    complexity: usize,
}

impl CognitiveVisitor {
    fn new() -> Self {
        Self {
            nesting: 0,
            complexity: 0,
        }
    }

    fn increment(&mut self) {
        // +1 for the construct, +nesting for nesting penalty
        self.complexity += 1 + self.nesting;
    }

    fn increment_no_nesting(&mut self) {
        self.complexity += 1;
    }

    fn count_boolean_chains(expr: &Expr) -> usize {
        match expr {
            Expr::Binary(bin) => {
                use syn::BinOp;
                let is_logical = matches!(bin.op, BinOp::And(_) | BinOp::Or(_));
                let left = Self::count_boolean_chains(&bin.left);
                let right = Self::count_boolean_chains(&bin.right);
                left + right + if is_logical { 1 } else { 0 }
            }
            _ => 0,
        }
    }

    /// Handle `if / else if / else` expressions for cognitive complexity.
    fn handle_if_expr(&mut self, expr_if: &syn::ExprIf) {
        self.increment();
        // Count boolean operators in condition
        self.complexity += Self::count_boolean_chains(&expr_if.cond);
        self.nesting += 1;
        // Visit then branch
        for stmt in &expr_if.then_branch.stmts {
            syn::visit::visit_stmt(self, stmt);
        }
        self.nesting -= 1;
        // Visit else branch
        if let Some((_, else_expr)) = &expr_if.else_branch {
            self.handle_else_branch(else_expr);
        }
    }

    /// Handle the else branch of an if expression.
    /// Matches original behavior: first else-if is handled inline with no nesting
    /// penalty, and deeper else branches delegate back to visit_expr.
    fn handle_else_branch(&mut self, else_expr: &Expr) {
        match else_expr {
            Expr::If(inner_if) => {
                // else if: +1 no nesting penalty (it's a continuation)
                self.increment_no_nesting();
                // But the inner if body still gets nesting
                self.complexity += Self::count_boolean_chains(&inner_if.cond);
                self.nesting += 1;
                for stmt in &inner_if.then_branch.stmts {
                    syn::visit::visit_stmt(self, stmt);
                }
                self.nesting -= 1;
                if let Some((_, inner_else)) = &inner_if.else_branch {
                    self.handle_inner_else(inner_else);
                }
            }
            Expr::Block(block) => {
                self.increment_no_nesting();
                self.nesting += 1;
                for stmt in &block.block.stmts {
                    syn::visit::visit_stmt(self, stmt);
                }
                self.nesting -= 1;
            }
            _ => {
                syn::visit::visit_expr(self, else_expr);
            }
        }
    }

    /// Handle deeper else branches (third level and beyond).
    /// Delegates Expr::If back to visit_expr for standard increment-with-nesting,
    /// matching the original behavior.
    fn handle_inner_else(&mut self, else_expr: &Expr) {
        match else_expr {
            Expr::If(_) => {
                self.visit_expr(else_expr);
            }
            Expr::Block(block) => {
                self.increment_no_nesting();
                self.nesting += 1;
                for stmt in &block.block.stmts {
                    syn::visit::visit_stmt(self, stmt);
                }
                self.nesting -= 1;
            }
            _ => {}
        }
    }

    /// Handle `match` expressions for cognitive complexity.
    fn handle_match_expr(&mut self, m: &syn::ExprMatch) {
        self.increment();
        self.nesting += 1;
        for arm in &m.arms {
            if arm.guard.is_some() {
                self.increment_no_nesting();
            }
            syn::visit::visit_expr(self, &arm.body);
        }
        self.nesting -= 1;
    }

    /// Handle `for`, `while`, and `loop` expressions for cognitive complexity.
    fn handle_loop_expr(&mut self, expr: &Expr) {
        self.increment();
        self.nesting += 1;
        syn::visit::visit_expr(self, expr);
        self.nesting -= 1;
    }
}

impl<'ast> Visit<'ast> for CognitiveVisitor {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        match expr {
            Expr::If(expr_if) => {
                self.handle_if_expr(expr_if);
                return;
            }
            Expr::Match(m) => {
                self.handle_match_expr(m);
                return;
            }
            Expr::ForLoop(_) | Expr::While(_) | Expr::Loop(_) => {
                self.handle_loop_expr(expr);
                return;
            }
            Expr::Break(_) | Expr::Continue(_) => {
                self.increment_no_nesting();
            }
            Expr::Closure(_) => {
                self.nesting += 1;
                syn::visit::visit_expr(self, expr);
                self.nesting -= 1;
                return;
            }
            _ => {}
        }
        syn::visit::visit_expr(self, expr);
    }
}

/// Compute complexity for all functions in the project.
pub fn compute_all(symbols: &ProjectSymbols, project_path: &Path) -> Vec<FunctionComplexity> {
    let mut results = Vec::new();

    for file_syms in &symbols.files {
        let source = std::fs::read_to_string(&file_syms.path).unwrap_or_default();

        for func in &file_syms.functions {
            let fc = compute_function_complexity(func, &source);
            results.push(FunctionComplexity {
                file: strip_project_path(&fc.file, project_path),
                ..fc
            });
        }
    }

    // Sort by composite score descending
    results.sort_by(|a, b| {
        b.composite_score
            .partial_cmp(&a.composite_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results
}

/// Compute per-file complexity aggregation.

/// Compute aggregate stats across all function complexities.
pub fn compute_stats(complexities: &[FunctionComplexity]) -> Option<ComplexityStats> {
    if complexities.is_empty() {
        return None;
    }

    let total = complexities.len();
    let scores: Vec<f64> = complexities.iter().map(|c| c.composite_score).collect();
    let sum: f64 = scores.iter().sum();
    let mean = sum / total as f64;
    let variance = scores.iter().map(|s| (s - mean) * (s - mean)).sum::<f64>() / total as f64;
    let std_dev = variance.sqrt();

    let mut sorted = scores.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = if total % 2 == 0 {
        (sorted[total / 2 - 1] + sorted[total / 2]) / 2.0
    } else {
        sorted[total / 2]
    };
    let max = sorted.last().copied().unwrap_or(0.0);

    Some(ComplexityStats {
        total_functions: total,
        mean,
        std_dev,
        median,
        max,
    })
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
