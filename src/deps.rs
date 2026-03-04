use colored::Colorize;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast_parser;
use crate::flow;
use crate::render;

/// An edge in the dependency graph: source module uses target module.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct DepEdge {
    pub from: String,
    pub to: String,
}

/// Per-module cohesion metrics.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleCohesion {
    pub module: String,
    pub call_cohesion: f64,
    pub type_cohesion: f64,
    pub combined: f64,
    pub function_count: usize,
}

/// Dependency analysis results.
pub struct DepAnalysis {
    pub modules: Vec<String>,
    pub edges: Vec<DepEdge>,
    pub in_degree: HashMap<String, usize>,
    pub out_degree: HashMap<String, usize>,
    pub coupling_pairs: Vec<(String, String, usize)>,
}

#[derive(Serialize)]
struct DepEdgeJson {
    from: String,
    to: String,
}

#[derive(Serialize)]
struct ModuleDegreeJson {
    module: String,
    in_degree: usize,
    out_degree: usize,
}

#[derive(Serialize)]
struct CouplingPairJson {
    module_a: String,
    module_b: String,
    strength: usize,
    direction: String,
}

#[derive(Serialize)]
struct DepsJson {
    cstat_version: String,
    modules: Vec<String>,
    edges: Vec<DepEdgeJson>,
    module_degrees: Vec<ModuleDegreeJson>,
    coupling_pairs: Vec<CouplingPairJson>,
    cohesion: Vec<ModuleCohesion>,
}


/// Extract use targets from a Rust source file.
/// Returns a list of module paths that this file imports from.
fn extract_use_targets(source: &str) -> Vec<String> {
    let ast = match syn::parse_file(source) {
        Ok(f) => f,
        Err(_) => return vec![],
    };

    let mut targets = Vec::new();
    for item in &ast.items {
        match item {
            syn::Item::Use(use_item) => {
                collect_use_paths(&use_item.tree, &mut vec![], &mut targets);
            }
            syn::Item::Mod(mod_item) => {
                // `mod foo;` declarations indicate a dependency on that module
                if mod_item.content.is_none() {
                    targets.push(mod_item.ident.to_string());
                }
            }
            _ => {}
        }
    }
    targets
}

/// Recursively collect paths from a use tree.
fn collect_use_paths(tree: &syn::UseTree, prefix: &mut Vec<String>, results: &mut Vec<String>) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            collect_use_paths(&p.tree, prefix, results);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            prefix.push(n.ident.to_string());
            results.push(prefix.join("::"));
            prefix.pop();
        }
        syn::UseTree::Rename(r) => {
            prefix.push(r.ident.to_string());
            results.push(prefix.join("::"));
            prefix.pop();
        }
        syn::UseTree::Glob(_) => {
            // use foo::* -> depends on foo
            results.push(prefix.join("::"));
        }
        syn::UseTree::Group(g) => {
            for item in &g.items {
                collect_use_paths(item, prefix, results);
            }
        }
    }
}

/// Match a use path to a known project module name.
/// For `use crate::foo::bar::Baz`, the module is likely `foo::bar` or `foo`.
fn resolve_to_module(use_path: &str, known_modules: &HashSet<String>) -> Option<String> {
    // Skip external crate imports (std, external deps)
    let path = if let Some(rest) = use_path.strip_prefix("crate::") {
        rest
    } else if let Some(rest) = use_path.strip_prefix("self::") {
        rest
    } else if let Some(rest) = use_path.strip_prefix("super::") {
        rest
    } else {
        use_path
    };

    // Try progressively shorter prefixes to match a module
    let parts: Vec<&str> = path.split("::").collect();
    for len in (1..=parts.len()).rev() {
        let candidate = parts[..len].join("::");
        if known_modules.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Build the full dependency analysis from discovered files.
pub fn analyze_deps(rs_files: &[PathBuf], project_path: &Path) -> DepAnalysis {
    // Build module name mapping
    let mut module_names: Vec<String> = Vec::new();
    let mut file_to_module: HashMap<PathBuf, String> = HashMap::new();

    for file in rs_files {
        let name = render::module_name_from_path(file, project_path);
        file_to_module.insert(file.clone(), name.clone());
        module_names.push(name);
    }

    let known_modules: HashSet<String> = module_names.iter().cloned().collect();

    // Build edges by analyzing use statements
    let mut edges: HashSet<DepEdge> = HashSet::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut out_degree: HashMap<String, usize> = HashMap::new();

    // Initialize all modules with 0 degree
    for name in &module_names {
        in_degree.insert(name.clone(), 0);
        out_degree.insert(name.clone(), 0);
    }

    for file in rs_files {
        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let from_module = match file_to_module.get(file) {
            Some(m) => m.clone(),
            None => continue,
        };

        let use_targets = extract_use_targets(&source);
        let mut seen_targets: HashSet<String> = HashSet::new();

        for target in &use_targets {
            if let Some(to_module) = resolve_to_module(target, &known_modules) {
                if to_module != from_module && seen_targets.insert(to_module.clone()) {
                    let edge = DepEdge {
                        from: from_module.clone(),
                        to: to_module.clone(),
                    };
                    if edges.insert(edge) {
                        *out_degree.entry(from_module.clone()).or_insert(0) += 1;
                        *in_degree.entry(to_module.clone()).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    // Compute coupling pairs: pairs with bidirectional edges, ranked by total edges
    let mut coupling_pairs: Vec<(String, String, usize)> = Vec::new();
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();

    for edge in &edges {
        let a = &edge.from;
        let b = &edge.to;
        let pair = if a < b {
            (a.clone(), b.clone())
        } else {
            (b.clone(), a.clone())
        };

        if seen_pairs.contains(&pair) {
            continue;
        }
        seen_pairs.insert(pair.clone());

        let fwd = edges.contains(&DepEdge {
            from: pair.0.clone(),
            to: pair.1.clone(),
        });
        let bwd = edges.contains(&DepEdge {
            from: pair.1.clone(),
            to: pair.0.clone(),
        });
        let strength = if fwd { 1 } else { 0 } + if bwd { 1 } else { 0 };
        if strength > 0 {
            coupling_pairs.push((pair.0, pair.1, strength));
        }
    }

    coupling_pairs.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));

    module_names.sort();

    DepAnalysis {
        modules: module_names,
        edges: edges.into_iter().collect(),
        in_degree,
        out_degree,
        coupling_pairs,
    }
}

/// Compute per-module cohesion metrics and attach them to the analysis.
pub fn compute_cohesion(rs_files: &[PathBuf], project_path: &Path) -> Vec<ModuleCohesion> {
    // Build call graph for call-cohesion
    let call_graph = flow::build_call_graph(rs_files, project_path);
    // Parse AST for type-cohesion
    let symbols = ast_parser::parse_project(rs_files);

    // Group functions by module (using the module prefix before "::")
    // call_graph.functions has names like "module::func" or bare "func" for main/lib
    let mut module_functions: HashMap<String, Vec<String>> = HashMap::new();
    for func_name in &call_graph.functions {
        let module = if let Some(pos) = func_name.rfind("::") {
            func_name[..pos].to_string()
        } else {
            "main".to_string()
        };
        module_functions.entry(module).or_default().push(func_name.clone());
    }

    // --- Call cohesion ---
    // For each module, fraction of functions that call at least one other function
    // in the same module.
    let mut call_cohesion_map: HashMap<String, f64> = HashMap::new();
    for (module, funcs) in &module_functions {
        if funcs.len() <= 1 {
            call_cohesion_map.insert(module.clone(), 1.0);
            continue;
        }
        let func_set: HashSet<&str> = funcs.iter().map(|s| s.as_str()).collect();
        let mut callers_of_internal = 0usize;
        for func in funcs {
            if let Some(callees) = call_graph.adjacency.get(func) {
                let calls_internal = callees.iter().any(|c| func_set.contains(c.as_str()) && c != func);
                if calls_internal {
                    callers_of_internal += 1;
                }
            }
        }
        call_cohesion_map.insert(module.clone(), callers_of_internal as f64 / funcs.len() as f64);
    }

    // --- Type cohesion ---
    // Group FunctionInfo by module. For each module, fraction of functions that share
    // at least one parameter type or return type with another function in the module.
    let mut module_func_infos: HashMap<String, Vec<&ast_parser::FunctionInfo>> = HashMap::new();
    for file_syms in &symbols.files {
        let module_name = render::module_name_from_path(&file_syms.path, project_path);
        for func in &file_syms.functions {
            // func.name might be "Type::method" for impl methods, map to module
            module_func_infos.entry(module_name.clone()).or_default().push(func);
        }
    }

    // Extract parameter type strings from source for each function
    let mut type_cohesion_map: HashMap<String, f64> = HashMap::new();
    for (module, funcs) in &module_func_infos {
        if funcs.len() <= 1 {
            type_cohesion_map.insert(module.clone(), 1.0);
            continue;
        }

        // For each function, collect a set of "type tokens" from its signature
        // We use the source file to extract actual type strings
        let func_types: Vec<HashSet<String>> = funcs.iter().map(|f| {
            extract_param_types(&f.file, f.line_start, f.line_end)
        }).collect();

        let mut sharing_count = 0usize;
        for (i, types_i) in func_types.iter().enumerate() {
            if types_i.is_empty() {
                continue;
            }
            let shares = func_types.iter().enumerate().any(|(j, types_j)| {
                i != j && !types_i.is_disjoint(types_j)
            });
            if shares {
                sharing_count += 1;
            }
        }
        type_cohesion_map.insert(module.clone(), sharing_count as f64 / funcs.len() as f64);
    }

    // --- Build combined results ---
    let all_modules: HashSet<&str> = module_functions.keys().map(|s| s.as_str())
        .chain(module_func_infos.keys().map(|s| s.as_str()))
        .collect();

    let mut results: Vec<ModuleCohesion> = all_modules.iter().map(|&module| {
        let call_c = call_cohesion_map.get(module).copied().unwrap_or(0.0);
        let type_c = type_cohesion_map.get(module).copied().unwrap_or(0.0);
        let func_count = module_functions.get(module).map(|v| v.len())
            .or_else(|| module_func_infos.get(module).map(|v| v.len()))
            .unwrap_or(0);
        // Combined: weighted average (call cohesion is stronger signal)
        let combined = (0.6 * call_c + 0.4 * type_c).clamp(0.0, 1.0);
        ModuleCohesion {
            module: module.to_string(),
            call_cohesion: call_c,
            type_cohesion: type_c,
            combined,
            function_count: func_count,
        }
    }).collect();

    results.sort_by(|a, b| a.combined.partial_cmp(&b.combined).unwrap_or(std::cmp::Ordering::Equal));
    results
}

/// Extract parameter/return type names from a function in a source file.
/// Returns a set of type-name tokens (stripped of generics, references, etc.).
fn extract_param_types(file: &Path, line_start: usize, line_end: usize) -> HashSet<String> {
    let content = match std::fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return HashSet::new(),
    };
    let lines: Vec<&str> = content.lines().collect();
    // Get the function signature area (first few lines until opening brace)
    let start = line_start.saturating_sub(1);
    let end = line_end.min(lines.len());
    let sig_text: String = lines[start..end].join("\n");

    // Parse type tokens: look for common Rust type patterns
    let mut types = HashSet::new();
    for token in sig_text.split(|c: char| !c.is_alphanumeric() && c != '_') {
        let t = token.trim();
        if t.is_empty() || t.len() < 2 {
            continue;
        }
        // Skip common keywords and lowercase-only short words (likely variable names)
        if matches!(t, "fn" | "pub" | "mut" | "self" | "let" | "if" | "else" | "for"
            | "while" | "match" | "return" | "use" | "crate" | "super" | "as" | "in"
            | "ref" | "where" | "impl" | "true" | "false" | "Some" | "None" | "Ok" | "Err") {
            continue;
        }
        // Keep type-like tokens (start with uppercase, or are well-known types)
        if t.starts_with(|c: char| c.is_uppercase())
            || matches!(t, "usize" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128"
                | "i8" | "i16" | "i32" | "i64" | "i128" | "f32" | "f64"
                | "bool" | "str" | "String") {
            types.insert(t.to_string());
        }
    }
    types
}

/// Render the deps subcommand output.
pub fn render_deps(rs_files: &[PathBuf], project_path: &Path, top_n: Option<usize>, verbose: bool) {
    let analysis = analyze_deps(rs_files, project_path);

    if analysis.modules.is_empty() {
        println!("{}", "No modules found to analyze.".yellow());
        return;
    }

    let term_w = render::terminal_width();
    let top = top_n.unwrap_or(15);

    // Section 1: Dependency Matrix
    render_dependency_matrix(&analysis, top, term_w, verbose);

    // Section 2: Coupling Ranking
    render_coupling_ranking(&analysis, top, verbose);

    // Section 3: Fan-in / Fan-out chart
    render_fan_chart(&analysis, top, term_w, verbose);

    // Section 4: Cohesion per module
    let cohesion = compute_cohesion(rs_files, project_path);
    render_cohesion_section(&cohesion, top, verbose);
}

/// Render a dependency heatmap matrix showing coupling intensity between modules.
/// Each cell contains a number: 0 = no dependency, 1 = unidirectional, 2 = bidirectional.
/// Colors: 0 = dim, 1 = yellow, 2 = red/bold. Diagonal is blank.
fn render_dependency_matrix(analysis: &DepAnalysis, top_n: usize, term_w: usize, verbose: bool) {
    println!(
        "\n{}",
        "── Dependency Heatmap ─────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Matrix showing coupling intensity between module pairs.",
            "Cell [row, col] = 0 (dim, no dependency), 1 (yellow, unidirectional),",
            "  2 (red/bold, bidirectional — both modules import each other).",
            "Diagonal is marked '·' (a module cannot depend on itself).",
            "Modules are sorted by total connectivity (in + out degree).",
            "Row/column indices map to module names shown in the legend below the matrix.",
            "Built from 'use' and 'mod' statements found by parsing each file's AST.",
        ]);
        render::guide_ref("modularity");
    }

    // Select most connected modules (by total in+out degree)
    let mut module_connectivity: Vec<(&String, usize)> = analysis
        .modules
        .iter()
        .map(|m| {
            let in_d = analysis.in_degree.get(m).copied().unwrap_or(0);
            let out_d = analysis.out_degree.get(m).copied().unwrap_or(0);
            (m, in_d + out_d)
        })
        .collect();
    module_connectivity.sort_by(|a, b| b.1.cmp(&a.1));

    let display_modules: Vec<&String> = module_connectivity
        .iter()
        .filter(|(_, c)| *c > 0)
        .take(top_n)
        .map(|(m, _)| *m)
        .collect();

    if display_modules.is_empty() {
        println!("  {}", "No inter-module dependencies detected.".dimmed());
        return;
    }

    let edge_set: HashSet<(&str, &str)> = analysis
        .edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();

    // Abbreviate module names for column headers
    let max_label = 20.min(
        display_modules
            .iter()
            .map(|m| m.len())
            .max()
            .unwrap_or(5),
    );

    // How much space we have for the matrix columns
    let matrix_start = max_label + 3; // label + " │ "
    let available = term_w.saturating_sub(matrix_start);
    let col_width = 3.min(available / display_modules.len().max(1)).max(2);

    // Truncate if too many modules for terminal width
    let max_cols = available / col_width.max(1);
    let display_count = display_modules.len().min(max_cols);
    let display_modules = &display_modules[..display_count];

    // Print column header indices
    println!();
    print!("{:>width$} │ ", "", width = max_label);
    for (i, _) in display_modules.iter().enumerate() {
        print!("{:>w$}", format!("{}", i), w = col_width);
    }
    println!();

    // Separator
    print!("{}", "─".repeat(max_label + 1));
    print!("┼");
    println!("{}", "─".repeat(display_modules.len() * col_width + 1));

    // Matrix rows — each cell is a numeric coupling intensity
    for (ri, row_mod) in display_modules.iter().enumerate() {
        let label = if row_mod.len() > max_label {
            format!("..{}", &row_mod[row_mod.len() - (max_label - 2)..])
        } else {
            (*row_mod).clone()
        };
        print!("{:>width$} │ ", label.dimmed(), width = max_label);

        for col_mod in display_modules {
            if *row_mod == *col_mod {
                // Diagonal: blank/dot
                let cell = "·".dimmed().to_string();
                // Pad to col_width accounting for ANSI codes
                let visible_len = 1; // "·" is 1 display char
                let pad = col_width.saturating_sub(visible_len);
                print!("{}{}", " ".repeat(pad), cell);
            } else {
                let has_fwd = edge_set.contains(&(row_mod.as_str(), col_mod.as_str()));
                let has_rev = edge_set.contains(&(col_mod.as_str(), row_mod.as_str()));
                let strength = (if has_fwd { 1 } else { 0 }) + (if has_rev { 1 } else { 0 });

                let cell = match strength {
                    0 => "0".dimmed().to_string(),
                    1 => "1".yellow().to_string(),
                    _ => "2".red().bold().to_string(),
                };
                // Pad to col_width accounting for ANSI codes — visible char is always 1
                let visible_len = 1;
                let pad = col_width.saturating_sub(visible_len);
                print!("{}{}", " ".repeat(pad), cell);
            }
        }
        println!("  {}", format!("{}", ri).dimmed());
    }

    // Legend
    println!();
    println!(
        "  {} no dependency  {} unidirectional  {} bidirectional",
        "0".dimmed(),
        "1".yellow(),
        "2".red().bold(),
    );

    // Module index legend
    println!();
    for (i, m) in display_modules.iter().enumerate() {
        println!("  {} = {}", format!("{:>2}", i).bold(), m.dimmed());
    }
}

/// Render coupling ranking — pairs of modules ordered by coupling strength.
fn render_coupling_ranking(analysis: &DepAnalysis, top_n: usize, verbose: bool) {
    println!(
        "\n{}",
        "── Coupling Ranking ───────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Module pairs ranked by coupling strength.",
            "◆ bidirectional (red) = both modules import each other. This is a coupling risk —",
            "  changes in either module may ripple to the other.",
            "→ unidirectional (yellow) = one-way dependency, lower risk.",
            "Pairs are sorted by coupling strength descending, then alphabetically.",
        ]);
    }

    if analysis.coupling_pairs.is_empty() {
        println!("  {}", "No module coupling detected.".dimmed());
        return;
    }

    let display_count = analysis.coupling_pairs.len().min(top_n);
    println!(
        "\n  {:<30} {:<30} {}",
        "Module A".bold().underline(),
        "Module B".bold().underline(),
        "Coupling".bold().underline(),
    );

    for (a, b, strength) in analysis.coupling_pairs.iter().take(display_count) {
        let coupling_label = if *strength >= 2 {
            "◆ bidirectional".red().bold().to_string()
        } else {
            "→ unidirectional".yellow().to_string()
        };

        let a_display = if a.len() > 28 {
            format!("..{}", &a[a.len() - 26..])
        } else {
            a.clone()
        };
        let b_display = if b.len() > 28 {
            format!("..{}", &b[b.len() - 26..])
        } else {
            b.clone()
        };

        println!("  {:<30} {:<30} {}", a_display.dimmed(), b_display.dimmed(), coupling_label);
    }

    if analysis.coupling_pairs.len() > display_count {
        println!(
            "{}",
            format!(
                "  ... and {} more pairs",
                analysis.coupling_pairs.len() - display_count
            )
            .dimmed()
        );
    }
}

/// Render fan-in/fan-out horizontal bar chart.
fn render_fan_chart(analysis: &DepAnalysis, top_n: usize, term_w: usize, verbose: bool) {
    println!(
        "\n{}",
        "── Fan-in / Fan-out ───────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Horizontal bar chart showing each module's connectivity profile.",
            "█ (blue) = fan-in: how many modules depend on this one.",
            "  High fan-in = core utility — many other modules import it.",
            "░ (yellow) = fan-out: how many modules this one depends on.",
            "  High fan-out = potential god module — it knows about everything.",
            "Modules sorted by total connectivity (fan-in + fan-out) descending.",
        ]);
    }

    // Sort modules by total connectivity
    let mut module_stats: Vec<(&String, usize, usize)> = analysis
        .modules
        .iter()
        .map(|m| {
            let in_d = analysis.in_degree.get(m).copied().unwrap_or(0);
            let out_d = analysis.out_degree.get(m).copied().unwrap_or(0);
            (m, in_d, out_d)
        })
        .filter(|(_, i, o)| *i > 0 || *o > 0)
        .collect();

    module_stats.sort_by(|a, b| (b.1 + b.2).cmp(&(a.1 + a.2)));

    if module_stats.is_empty() {
        println!("  {}", "No inter-module dependencies detected.".dimmed());
        return;
    }

    let display_count = module_stats.len().min(top_n);
    let display_stats = &module_stats[..display_count];

    let max_label_len = display_stats
        .iter()
        .map(|(m, _, _)| m.len())
        .max()
        .unwrap_or(5)
        .min(25);

    let max_degree = display_stats
        .iter()
        .map(|(_, i, o)| *i + *o)
        .max()
        .unwrap_or(1);

    // Budget: label + " │ " + bar area + " in:N out:N"
    let suffix_len = 14; // " in:NN out:NN"
    let bar_budget = term_w
        .saturating_sub(max_label_len)
        .saturating_sub(3) // " │ "
        .saturating_sub(suffix_len)
        .max(10);

    println!(
        "\n{:>width$}   {}{}",
        "",
        "█ fan-in".bright_blue(),
        "  ░ fan-out".yellow(),
        width = max_label_len,
    );
    println!();

    for (name, in_d, out_d) in display_stats {
        let label = if name.len() > max_label_len {
            format!("..{}", &name[name.len() - (max_label_len - 2)..])
        } else {
            (*name).clone()
        };

        let total = in_d + out_d;
        let total_bar = if max_degree > 0 {
            (total as f64 / max_degree as f64 * bar_budget as f64).ceil() as usize
        } else {
            0
        }
        .max(if total > 0 { 1 } else { 0 });

        let in_bar = if total > 0 {
            ((*in_d as f64 / total as f64) * total_bar as f64).round() as usize
        } else {
            0
        };
        let out_bar = total_bar.saturating_sub(in_bar);

        let in_str = "█".repeat(in_bar).bright_blue().to_string();
        let out_str = "░".repeat(out_bar).yellow().to_string();
        let padding = " ".repeat(bar_budget.saturating_sub(total_bar));

        println!(
            "{:>width$} │ {}{}{} in:{} out:{}",
            label.dimmed(),
            in_str,
            out_str,
            padding,
            format!("{}", in_d).bright_blue(),
            format!("{}", out_d).yellow(),
            width = max_label_len,
        );
    }

    if module_stats.len() > display_count {
        println!(
            "{}",
            format!(
                "  ... and {} more modules",
                module_stats.len() - display_count
            )
            .dimmed()
        );
    }

    println!();
}

/// Render cohesion scores per module.
fn render_cohesion_section(cohesion: &[ModuleCohesion], top_n: usize, verbose: bool) {
    println!(
        "\n{}",
        "── Module Cohesion ───────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Per-module cohesion scores measuring internal focus/coherence.",
            "Call cohesion: fraction of functions calling another function in the same module.",
            "Type cohesion: fraction of functions sharing parameter/return types within the module.",
            "Combined: weighted average (60% call + 40% type), 0.0–1.0 where higher = more cohesive.",
            "Low cohesion suggests the module bundles unrelated functionality.",
            "Modules sorted by combined cohesion ascending (least cohesive first).",
        ]);
    }

    // Filter to modules with more than 1 function (cohesion is trivially 1.0 for singletons)
    let display: Vec<&ModuleCohesion> = cohesion.iter()
        .filter(|c| c.function_count > 1)
        .take(top_n)
        .collect();

    if display.is_empty() {
        println!("  {}", "No multi-function modules to measure cohesion.".dimmed());
        return;
    }

    let max_label_len = display.iter().map(|c| c.module.len()).max().unwrap_or(5).min(25);

    println!(
        "\n  {:>width$}   {:<8} {:<8} {}",
        "Module".bold().underline(),
        "Call".bold().underline(),
        "Type".bold().underline(),
        "Combined".bold().underline(),
        width = max_label_len,
    );

    for mc in &display {
        let label = if mc.module.len() > max_label_len {
            format!("..{}", &mc.module[mc.module.len() - (max_label_len - 2)..])
        } else {
            mc.module.clone()
        };

        let combined_colored = if mc.combined < 0.1 {
            format!("{:.2}", mc.combined).red().bold().to_string()
        } else if mc.combined < 0.3 {
            format!("{:.2}", mc.combined).yellow().to_string()
        } else {
            format!("{:.2}", mc.combined).green().to_string()
        };

        println!(
            "  {:>width$}   {:<8} {:<8} {}  ({} fns)",
            label.dimmed(),
            format!("{:.2}", mc.call_cohesion),
            format!("{:.2}", mc.type_cohesion),
            combined_colored,
            mc.function_count,
            width = max_label_len,
        );
    }

    if cohesion.iter().filter(|c| c.function_count > 1).count() > display.len() {
        println!(
            "{}",
            format!(
                "  ... and {} more modules",
                cohesion.iter().filter(|c| c.function_count > 1).count() - display.len()
            )
            .dimmed()
        );
    }

    println!();
}

/// Render dependency analysis as JSON.
pub fn render_deps_json(rs_files: &[PathBuf], project_path: &Path) {
    let analysis = analyze_deps(rs_files, project_path);

    let edges: Vec<DepEdgeJson> = analysis
        .edges
        .iter()
        .map(|e| DepEdgeJson { from: e.from.clone(), to: e.to.clone() })
        .collect();

    let module_degrees: Vec<ModuleDegreeJson> = analysis
        .modules
        .iter()
        .map(|m| ModuleDegreeJson {
            module: m.clone(),
            in_degree: analysis.in_degree.get(m).copied().unwrap_or(0),
            out_degree: analysis.out_degree.get(m).copied().unwrap_or(0),
        })
        .collect();

    let coupling_pairs: Vec<CouplingPairJson> = analysis
        .coupling_pairs
        .iter()
        .map(|(a, b, s)| CouplingPairJson {
            module_a: a.clone(),
            module_b: b.clone(),
            strength: *s,
            direction: if *s >= 2 { "bidirectional".to_string() } else { "unidirectional".to_string() },
        })
        .collect();

    let cohesion = compute_cohesion(rs_files, project_path);

    let output = DepsJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        modules: analysis.modules,
        edges,
        module_degrees,
        coupling_pairs,
        cohesion,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}
