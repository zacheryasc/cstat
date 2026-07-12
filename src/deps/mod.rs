mod render;

pub use render::{render_deps, render_deps_json};

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast_parser;
use crate::flow;
use crate::render as crate_render;

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

/// Extract use targets from a Rust source file.
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
fn resolve_to_module(use_path: &str, known_modules: &HashSet<String>) -> Option<String> {
    let path = if let Some(rest) = use_path.strip_prefix("crate::") {
        rest
    } else if let Some(rest) = use_path.strip_prefix("self::") {
        rest
    } else if let Some(rest) = use_path.strip_prefix("super::") {
        rest
    } else {
        use_path
    };

    let parts: Vec<&str> = path.split("::").collect();
    for len in (1..=parts.len()).rev() {
        let candidate = parts[..len].join("::");
        if known_modules.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Compute coupling pairs from the edge set, sorted by strength descending.
fn build_coupling_pairs(edges: &HashSet<DepEdge>) -> Vec<(String, String, usize)> {
    let mut coupling_pairs: Vec<(String, String, usize)> = Vec::new();
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();

    for edge in edges {
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
    coupling_pairs
}

/// Build the full dependency analysis from discovered files.
pub fn analyze_deps(rs_files: &[PathBuf], project_path: &Path) -> DepAnalysis {
    let mut module_names: Vec<String> = Vec::new();
    let mut file_to_module: HashMap<PathBuf, String> = HashMap::new();

    for file in rs_files {
        let name = crate_render::module_name_from_path(file, project_path);
        file_to_module.insert(file.clone(), name.clone());
        module_names.push(name);
    }

    let known_modules: HashSet<String> = module_names.iter().cloned().collect();

    let mut edges: HashSet<DepEdge> = HashSet::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut out_degree: HashMap<String, usize> = HashMap::new();

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

    let coupling_pairs = build_coupling_pairs(&edges);

    module_names.sort();

    DepAnalysis {
        modules: module_names,
        edges: edges.into_iter().collect(),
        in_degree,
        out_degree,
        coupling_pairs,
    }
}

/// Compute call cohesion: fraction of functions calling another function in the same module.
fn compute_call_cohesion(
    module_functions: &HashMap<String, Vec<String>>,
    call_graph: &flow::CallGraph,
) -> HashMap<String, f64> {
    let mut call_cohesion_map: HashMap<String, f64> = HashMap::new();
    for (module, funcs) in module_functions {
        if funcs.len() <= 1 {
            call_cohesion_map.insert(module.clone(), 1.0);
            continue;
        }
        let func_set: HashSet<&str> = funcs.iter().map(|s| s.as_str()).collect();
        let mut callers_of_internal = 0usize;
        for func in funcs {
            if let Some(callees) = call_graph.adjacency.get(func) {
                let calls_internal = callees
                    .iter()
                    .any(|c| func_set.contains(c.as_str()) && c != func);
                if calls_internal {
                    callers_of_internal += 1;
                }
            }
        }
        call_cohesion_map.insert(
            module.clone(),
            callers_of_internal as f64 / funcs.len() as f64,
        );
    }
    call_cohesion_map
}

/// Compute type cohesion: fraction of functions sharing parameter/return types within a module.
fn compute_type_cohesion(
    module_func_infos: &HashMap<String, Vec<&ast_parser::FunctionInfo>>,
) -> HashMap<String, f64> {
    let mut type_cohesion_map: HashMap<String, f64> = HashMap::new();
    for (module, funcs) in module_func_infos {
        if funcs.len() <= 1 {
            type_cohesion_map.insert(module.clone(), 1.0);
            continue;
        }

        let func_types: Vec<HashSet<String>> = funcs
            .iter()
            .map(|f| extract_param_types(&f.file, f.line_start, f.line_end))
            .collect();

        let mut sharing_count = 0usize;
        for (i, types_i) in func_types.iter().enumerate() {
            if types_i.is_empty() {
                continue;
            }
            let shares = func_types
                .iter()
                .enumerate()
                .any(|(j, types_j)| i != j && !types_i.is_disjoint(types_j));
            if shares {
                sharing_count += 1;
            }
        }
        type_cohesion_map.insert(module.clone(), sharing_count as f64 / funcs.len() as f64);
    }
    type_cohesion_map
}

/// Compute per-module cohesion metrics.
pub fn compute_cohesion(rs_files: &[PathBuf], project_path: &Path) -> Vec<ModuleCohesion> {
    let call_graph = flow::build_call_graph(rs_files, project_path);
    let symbols = ast_parser::parse_project(rs_files);

    let mut module_functions: HashMap<String, Vec<String>> = HashMap::new();
    for func_name in &call_graph.functions {
        let module = if let Some(pos) = func_name.rfind("::") {
            func_name[..pos].to_string()
        } else {
            "main".to_string()
        };
        module_functions
            .entry(module)
            .or_default()
            .push(func_name.clone());
    }

    let call_cohesion_map = compute_call_cohesion(&module_functions, &call_graph);

    let mut module_func_infos: HashMap<String, Vec<&ast_parser::FunctionInfo>> = HashMap::new();
    for file_syms in &symbols.files {
        let module_name = crate_render::module_name_from_path(&file_syms.path, project_path);
        for func in &file_syms.functions {
            module_func_infos
                .entry(module_name.clone())
                .or_default()
                .push(func);
        }
    }

    let type_cohesion_map = compute_type_cohesion(&module_func_infos);

    let all_modules: HashSet<&str> = module_functions
        .keys()
        .map(|s| s.as_str())
        .chain(module_func_infos.keys().map(|s| s.as_str()))
        .collect();

    let mut results: Vec<ModuleCohesion> = all_modules
        .iter()
        .map(|&module| {
            let call_c = call_cohesion_map.get(module).copied().unwrap_or(0.0);
            let type_c = type_cohesion_map.get(module).copied().unwrap_or(0.0);
            let func_count = module_functions
                .get(module)
                .map(|v| v.len())
                .or_else(|| module_func_infos.get(module).map(|v| v.len()))
                .unwrap_or(0);
            let combined = (0.6 * call_c + 0.4 * type_c).clamp(0.0, 1.0);
            ModuleCohesion {
                module: module.to_string(),
                call_cohesion: call_c,
                type_cohesion: type_c,
                combined,
                function_count: func_count,
            }
        })
        .collect();

    results.sort_by(|a, b| {
        a.combined
            .partial_cmp(&b.combined)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results
}

/// Extract parameter/return type names from a function in a source file.
fn extract_param_types(file: &Path, line_start: usize, line_end: usize) -> HashSet<String> {
    let content = match std::fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return HashSet::new(),
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = line_start.saturating_sub(1);
    let end = line_end.min(lines.len());
    let sig_text: String = lines[start..end].join("\n");

    let mut types = HashSet::new();
    for token in sig_text.split(|c: char| !c.is_alphanumeric() && c != '_') {
        let t = token.trim();
        if t.is_empty() || t.len() < 2 {
            continue;
        }
        if matches!(
            t,
            "fn" | "pub"
                | "mut"
                | "self"
                | "let"
                | "if"
                | "else"
                | "for"
                | "while"
                | "match"
                | "return"
                | "use"
                | "crate"
                | "super"
                | "as"
                | "in"
                | "ref"
                | "where"
                | "impl"
                | "true"
                | "false"
                | "Some"
                | "None"
                | "Ok"
                | "Err"
        ) {
            continue;
        }
        if t.starts_with(|c: char| c.is_uppercase())
            || matches!(
                t,
                "usize"
                    | "isize"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "u128"
                    | "i8"
                    | "i16"
                    | "i32"
                    | "i64"
                    | "i128"
                    | "f32"
                    | "f64"
                    | "bool"
                    | "str"
                    | "String"
            )
        {
            types.insert(t.to_string());
        }
    }
    types
}
