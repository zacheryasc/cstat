use colored::Colorize;
use serde::Serialize;
use std::path::Path;
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
    pub generic_complexity: usize,
    pub param_complexity: usize,
    pub composite_score: f64,
    pub line_count: usize,
    pub param_count: usize,
}

/// Complexity summary for a single file.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FileComplexity {
    pub path: String,
    pub function_count: usize,
    pub total_complexity: f64,
    pub mean_complexity: f64,
    pub max_complexity: f64,
    pub total_generics: usize,
    pub trait_impl_count: usize,
    pub derive_density: f64,
}

/// Aggregate stats across all functions.
pub struct ComplexityStats {
    pub total_functions: usize,
    pub mean: f64,
    pub std_dev: f64,
    pub median: f64,
    pub max: f64,
}

#[derive(Serialize)]
struct FunctionComplexityJson {
    name: String,
    file: String,
    cyclomatic: usize,
    cognitive: usize,
    nesting_depth: usize,
    generic_complexity: usize,
    param_complexity: usize,
    composite_score: f64,
    line_count: usize,
    param_count: usize,
}

#[derive(Serialize)]
struct FileComplexityJson {
    path: String,
    function_count: usize,
    total_complexity: f64,
    mean_complexity: f64,
    max_complexity: f64,
    total_generics: usize,
    trait_impl_count: usize,
    derive_density: f64,
}

#[derive(Serialize)]
struct ComplexityStatsJson {
    total_functions: usize,
    mean: f64,
    std_dev: f64,
    median: f64,
    max: f64,
}

#[derive(Serialize)]
struct ComplexityJson {
    cstat_version: String,
    functions: Vec<FunctionComplexityJson>,
    files: Vec<FileComplexityJson>,
    stats: Option<ComplexityStatsJson>,
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

    let line_count = if func.line_end >= func.line_start {
        func.line_end - func.line_start + 1
    } else {
        func.body_stmt_count.max(1)
    };

    FunctionComplexity {
        name: func.name.clone(),
        file: func.file.display().to_string(),
        cyclomatic,
        cognitive,
        nesting_depth: func.nesting_depth,
        generic_complexity,
        param_complexity,
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
pub fn compute_file_complexity(
    symbols: &ProjectSymbols,
    func_complexities: &[FunctionComplexity],
    project_path: &Path,
) -> Vec<FileComplexity> {
    let mut file_map: std::collections::HashMap<String, Vec<&FunctionComplexity>> =
        std::collections::HashMap::new();

    for fc in func_complexities {
        file_map.entry(fc.file.clone()).or_default().push(fc);
    }

    let mut file_complexities: Vec<FileComplexity> = symbols
        .files
        .iter()
        .map(|file_syms| {
            let path = strip_project_path(&file_syms.path.display().to_string(), project_path);
            let funcs = file_map.get(&path).cloned().unwrap_or_default();
            let function_count = funcs.len();
            let scores: Vec<f64> = funcs.iter().map(|f| f.composite_score).collect();
            let total_complexity: f64 = scores.iter().sum();
            let mean_complexity = if function_count > 0 {
                total_complexity / function_count as f64
            } else {
                0.0
            };
            let max_complexity = scores.iter().cloned().fold(0.0f64, f64::max);

            let total_generics: usize = file_syms
                .structs
                .iter()
                .map(|s| s.generic_param_count)
                .sum::<usize>()
                + file_syms
                    .enums
                    .iter()
                    .map(|e| e.generic_param_count)
                    .sum::<usize>();
            let trait_impl_count = file_syms
                .impls
                .iter()
                .filter(|i| i.trait_name.is_some())
                .count();
            let total_types = file_syms.structs.len() + file_syms.enums.len();
            let total_derives: usize = file_syms
                .structs
                .iter()
                .map(|s| s.derive_count)
                .sum::<usize>()
                + file_syms
                    .enums
                    .iter()
                    .map(|e| e.derive_count)
                    .sum::<usize>();
            let derive_density = if total_types > 0 {
                total_derives as f64 / total_types as f64
            } else {
                0.0
            };

            FileComplexity {
                path,
                function_count,
                total_complexity,
                mean_complexity,
                max_complexity,
                total_generics,
                trait_impl_count,
                derive_density,
            }
        })
        .collect();

    file_complexities.sort_by(|a, b| {
        b.total_complexity
            .partial_cmp(&a.total_complexity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    file_complexities
}

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

/// Render the function complexity rankings table.
fn render_function_rankings(func_complexities: &[FunctionComplexity], _term_w: usize, verbose: bool) {
    println!(
        "\n{}",
        "\u{2500}\u{2500} Function Complexity Rankings \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Functions ranked by composite complexity score (highest = most complex).",
            "Each column is an individual complexity metric:",
            "  Cycl = cyclomatic complexity = number of linearly independent paths",
            "         through the function (branch points + 1). Higher = more branches.",
            "  Cogn = cognitive complexity (SonarSource model): +1 per control flow break",
            "         (if, for, while, match, &&, ||), plus a nesting penalty per level.",
            "  Nest = deepest nesting depth in the function body.",
            "  Gen  = generic complexity = type params + trait bounds + where predicates.",
            "  Param = parameter count + return type complexity.",
            "  Score = composite: 1.0\u{00d7}Cycl + 0.5\u{00d7}Cogn + 0.3\u{00d7}Nest + 0.2\u{00d7}Gen + 0.1\u{00d7}Param.",
            "Color: red = highest score, green = lowest. Functions sorted descending by Score.",
        ]);
        render::guide_ref("complexity");
    }

    let display_count = func_complexities.len().min(20);
    println!(
        "{} functions analyzed, showing top {}\n",
        format!("{}", func_complexities.len()).bold(),
        format!("{}", display_count).bold(),
    );

    // Header
    println!(
        "  {:<40} {:>5} {:>5} {:>5} {:>5} {:>5} {:>7}",
        "Function".bold().underline(),
        "Cycl".bold().underline(),
        "Cogn".bold().underline(),
        "Nest".bold().underline(),
        "Gen".bold().underline(),
        "Param".bold().underline(),
        "Score".bold().underline(),
    );

    for (i, fc) in func_complexities.iter().take(display_count).enumerate() {
        let ratio = i as f64 / display_count.max(1) as f64;
        let name_display = if fc.name.len() > 38 {
            format!("{}...", &fc.name[..35])
        } else {
            fc.name.clone()
        };

        let score_str = format!("{:.1}", fc.composite_score);
        let colored_score = render::bar_color(&score_str, ratio);

        println!(
            "  {:<40} {:>5} {:>5} {:>5} {:>5} {:>5} {}",
            name_display.dimmed(),
            fc.cyclomatic,
            fc.cognitive,
            fc.nesting_depth,
            fc.generic_complexity,
            fc.param_complexity,
            colored_score,
        );
    }

    if func_complexities.len() > display_count {
        println!(
            "{}",
            format!(
                "  ... and {} more functions",
                func_complexities.len() - display_count
            )
            .dimmed()
        );
    }
}

/// Render the per-file complexity bar chart.
fn render_file_complexity_chart(file_complexities: &[FileComplexity], term_w: usize, verbose: bool) {
    println!(
        "\n{}",
        "\u{2500}\u{2500} Per-File Complexity \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Total composite complexity score per file (sum of all function scores in the file).",
            "Bar length is proportional to the file with the highest total score.",
            "Color: red = top of ranking, green = bottom. Files sorted descending.",
        ]);
    }

    let file_display_count = file_complexities.len().min(20);
    let display_files = &file_complexities[..file_display_count];

    let max_label_len = display_files
        .iter()
        .map(|f| f.path.len())
        .max()
        .unwrap_or(0)
        .min(40);

    let max_score = display_files
        .iter()
        .map(|f| f.total_complexity)
        .fold(0.0f64, f64::max);

    let score_digits = format!("{:.1}", max_score).len();
    let bar_budget = term_w
        .saturating_sub(max_label_len)
        .saturating_sub(3) // " \u{2502} "
        .saturating_sub(score_digits)
        .saturating_sub(2)
        .max(10);

    println!();
    for (i, fc) in display_files.iter().enumerate() {
        let ratio = i as f64 / file_display_count.max(1) as f64;
        let label = if fc.path.len() > 40 {
            format!("...{}", &fc.path[fc.path.len() - 37..])
        } else {
            fc.path.clone()
        };

        let bar_len = if max_score > 0.0 {
            (fc.total_complexity / max_score * bar_budget as f64).ceil() as usize
        } else {
            0
        }
        .max(if fc.total_complexity > 0.0 { 1 } else { 0 });

        let bar_str = "\u{2588}".repeat(bar_len);
        let colored_bar = render::bar_color(&bar_str, ratio);

        println!(
            "{:>width$} \u{2502} {}{} {}",
            label.dimmed(),
            colored_bar,
            " ".repeat(bar_budget.saturating_sub(bar_len)),
            format!("{:.1}", fc.total_complexity).bold(),
            width = max_label_len,
        );
    }

    if file_complexities.len() > file_display_count {
        println!(
            "{}",
            format!(
                "  ... and {} more files",
                file_complexities.len() - file_display_count
            )
            .dimmed()
        );
    }
}

/// Render the aggregate complexity statistics box.
fn render_complexity_stats(stats: &ComplexityStats, verbose: bool) {
    println!(
        "\n{}",
        "\u{2500}\u{2500} Complexity Statistics \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Aggregate statistics for composite complexity across all functions.",
            "Mean = average score. Std Dev = spread. Median = middle value (robust to outliers).",
            "Max = highest individual function score in the project.",
        ]);
    }

    println!("\u{250c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{252c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2510}");
    render::print_stat_row_wide("Total functions", &format!("{}", stats.total_functions));
    render::print_stat_row_wide("Mean score", &format!("{:.2}", stats.mean));
    render::print_stat_row_wide("Std Dev", &format!("{:.2}", stats.std_dev));
    render::print_stat_row_wide("Median score", &format!("{:.2}", stats.median));
    render::print_stat_row_wide("Max score", &format!("{:.2}", stats.max));
    println!("\u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2534}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2518}");
}

/// Render the complexity subcommand output.
pub fn render_complexity(symbols: &ProjectSymbols, project_path: &Path, verbose: bool) {
    let func_complexities = compute_all(symbols, project_path);

    if func_complexities.is_empty() {
        println!("{}", "No functions found to analyze.".yellow());
        return;
    }

    let file_complexities = compute_file_complexity(symbols, &func_complexities, project_path);
    let stats = compute_stats(&func_complexities);
    let term_w = render::terminal_width();

    render_function_rankings(&func_complexities, term_w, verbose);
    render_file_complexity_chart(&file_complexities, term_w, verbose);

    if let Some(ref stats) = stats {
        render_complexity_stats(stats, verbose);
    }

    println!();
}

/// Render complexity analysis as JSON.
pub fn render_complexity_json(symbols: &ProjectSymbols, project_path: &Path) {
    let func_complexities = compute_all(symbols, project_path);
    let file_complexities = compute_file_complexity(symbols, &func_complexities, project_path);
    let stats = compute_stats(&func_complexities);

    let functions: Vec<FunctionComplexityJson> = func_complexities
        .iter()
        .map(|fc| FunctionComplexityJson {
            name: fc.name.clone(),
            file: fc.file.clone(),
            cyclomatic: fc.cyclomatic,
            cognitive: fc.cognitive,
            nesting_depth: fc.nesting_depth,
            generic_complexity: fc.generic_complexity,
            param_complexity: fc.param_complexity,
            composite_score: fc.composite_score,
            line_count: fc.line_count,
            param_count: fc.param_count,
        })
        .collect();

    let files: Vec<FileComplexityJson> = file_complexities
        .iter()
        .map(|fc| FileComplexityJson {
            path: fc.path.clone(),
            function_count: fc.function_count,
            total_complexity: fc.total_complexity,
            mean_complexity: fc.mean_complexity,
            max_complexity: fc.max_complexity,
            total_generics: fc.total_generics,
            trait_impl_count: fc.trait_impl_count,
            derive_density: fc.derive_density,
        })
        .collect();

    let stats_json = stats.map(|s| ComplexityStatsJson {
        total_functions: s.total_functions,
        mean: s.mean,
        std_dev: s.std_dev,
        median: s.median,
        max: s.max,
    });

    let output = ComplexityJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        functions,
        files,
        stats: stats_json,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}


pub fn run(rs_files: &[std::path::PathBuf], project_path: &Path, json: bool, verbose: bool) {
    let symbols = crate::ast_parser::parse_project(rs_files);
    if json {
        render_complexity_json(&symbols, project_path);
    } else {
        render_complexity(&symbols, project_path, verbose);
    }
}