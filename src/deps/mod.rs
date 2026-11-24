mod render;

pub use render::{render_deps, render_deps_json};

use colored::Colorize;
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

#[derive(Debug, Clone, Serialize)]
pub struct FileDependency {
    pub raw: String,
    pub resolved_module: Option<String>,
    pub kind: FileDependencyKind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileDependencyKind {
    Internal,
    External,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileDependencyReport {
    pub file: String,
    pub module: String,
    pub imports: usize,
    pub internal: Vec<FileDependency>,
    pub external: Vec<FileDependency>,
}

struct ModuleIndex<'a> {
    known: HashMap<&'a str, &'a Path>,
    unscored: HashSet<String>,
}

impl<'a> ModuleIndex<'a> {
    fn new(module_names: &'a [String], files: &'a [PathBuf]) -> Self {
        Self {
            known: module_names
                .iter()
                .map(String::as_str)
                .zip(files.iter().map(PathBuf::as_path))
                .collect(),
            unscored: HashSet::new(),
        }
    }

    fn read_imports(&mut self, file: &Path, module: &str, collect_uses: bool) -> Vec<String> {
        let Ok(source) = std::fs::read_to_string(file) else {
            return Vec::new();
        };
        let Ok(ast) = syn::parse_file(&source) else {
            return Vec::new();
        };

        let mut targets = Vec::new();
        for item in &ast.items {
            match item {
                syn::Item::Use(use_item) if collect_uses => {
                    collect_use_paths(&use_item.tree, &mut vec![], &mut targets);
                }
                syn::Item::Mod(mod_item) if mod_item.content.is_none() => {
                    let name = mod_item.ident.to_string();
                    let child = format!("{module}::{name}");
                    if !self.known.contains_key(child.as_str()) {
                        self.unscored.insert(child);
                    }
                    if collect_uses {
                        targets.push(name);
                    }
                }
                _ => {}
            }
        }
        targets
    }
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

/// Match a use/mod path to a known project module name from a source-module context.
fn resolve_to_module<'a>(
    use_path: &str,
    modules: &ModuleIndex<'a>,
    current_module: &str,
    crate_base: &str,
) -> Option<&'a str> {
    if use_path == "crate" {
        return None;
    }
    if use_path == "self" {
        return modules
            .known
            .get_key_value(current_module)
            .map(|(&name, _)| name);
    }
    if let Some(rest) = use_path.strip_prefix("crate::") {
        return resolve_relative_module_path(crate_base, rest, modules);
    }
    if let Some(rest) = use_path.strip_prefix("self::") {
        return resolve_relative_module_path(current_module, rest, modules);
    }

    if use_path == "super" || use_path.starts_with("super::") {
        let mut base = current_module;
        let mut rest = use_path;
        while rest == "super" || rest.starts_with("super::") {
            if base == crate_base {
                return None;
            }
            base = parent_module(base).unwrap_or(crate_base);
            rest = rest.strip_prefix("super::").unwrap_or("");
        }
        return resolve_relative_module_path(base, rest, modules);
    }

    resolve_relative_module_path(current_module, use_path, modules)
        .or_else(|| resolve_absolute_module_path(use_path, modules))
}

fn resolve_absolute_module_path<'a>(mut path: &str, modules: &ModuleIndex<'a>) -> Option<&'a str> {
    loop {
        if let Some((&module, _)) = modules.known.get_key_value(path) {
            return Some(module);
        }
        // An excluded file module is still a module, not an item in its parent.
        if modules.unscored.contains(path) {
            return None;
        }
        path = parent_module(path)?;
    }
}

fn resolve_relative_module_path<'a>(
    base: &str,
    path: &str,
    modules: &ModuleIndex<'a>,
) -> Option<&'a str> {
    if base.is_empty() {
        resolve_absolute_module_path(path, modules)
    } else if path.is_empty() {
        resolve_absolute_module_path(base, modules)
    } else {
        resolve_absolute_module_path(&format!("{base}::{path}"), modules)
    }
}

fn crate_module_base<'a>(module: &'a str, file: &Path, project_path: &Path) -> &'a str {
    let relative = file.strip_prefix(project_path).unwrap_or(file);
    match relative
        .components()
        .position(|part| part.as_os_str() == "src")
    {
        // The root crate's `src` is stripped from module IDs; workspace members retain it.
        Some(depth) if depth > 0 => module
            .match_indices("::")
            .nth(depth)
            .map_or("", |(index, _)| &module[..index]),
        _ => "",
    }
}

fn parent_module(module: &str) -> Option<&str> {
    module.rsplit_once("::").map(|(parent, _)| parent)
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
    let mut module_names: Vec<_> = rs_files
        .iter()
        .map(|file| crate_render::module_name_from_path(file, project_path))
        .collect();
    let mut modules = ModuleIndex::new(&module_names, rs_files);
    let imports: Vec<_> = rs_files
        .iter()
        .zip(&module_names)
        .map(|(file, module)| modules.read_imports(file, module, true))
        .collect();

    let mut edges: HashSet<DepEdge> = HashSet::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut out_degree: HashMap<String, usize> = HashMap::new();

    for name in &module_names {
        in_degree.insert(name.clone(), 0);
        out_degree.insert(name.clone(), 0);
    }

    for ((file, from_module), targets) in rs_files.iter().zip(&module_names).zip(imports) {
        let crate_base = crate_module_base(from_module, file, project_path);
        let mut seen_targets = HashSet::new();
        for target in targets {
            if let Some(to_module) = resolve_to_module(&target, &modules, from_module, crate_base) {
                if to_module != from_module && seen_targets.insert(to_module) {
                    let edge = DepEdge {
                        from: from_module.clone(),
                        to: to_module.to_string(),
                    };
                    if edges.insert(edge) {
                        *out_degree.get_mut(from_module).unwrap() += 1;
                        *in_degree.get_mut(to_module).unwrap() += 1;
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

pub fn analyze_file_deps(
    rs_files: &[PathBuf],
    project_path: &Path,
    file: &Path,
) -> FileDependencyReport {
    let module_names: Vec<_> = rs_files
        .iter()
        .map(|file| crate_render::module_name_from_path(file, project_path))
        .collect();
    let mut modules = ModuleIndex::new(&module_names, rs_files);

    let module = crate_render::module_name_from_path(file, project_path);
    let crate_base = crate_module_base(&module, file, project_path);
    let file_display = relative_path_string(project_path, file);
    let mut use_targets = modules.read_imports(file, &module, true);
    use_targets.sort();
    use_targets.dedup();
    let mut indexed = HashSet::from([module.as_str()]);

    let mut internal = Vec::new();
    let mut external = Vec::new();
    for raw in use_targets {
        let resolved_module = loop {
            let Some(candidate) = resolve_to_module(&raw, &modules, &module, crate_base) else {
                break None;
            };
            if !indexed.insert(candidate) {
                break Some(candidate);
            }
            // For a file report, only read declarations from potential targets.
            let source = modules.known[candidate];
            modules.read_imports(source, candidate, false);
        };
        if let Some(resolved_module) = resolved_module {
            internal.push(FileDependency {
                raw,
                resolved_module: Some(resolved_module.to_string()),
                kind: FileDependencyKind::Internal,
            });
        } else {
            external.push(FileDependency {
                raw,
                resolved_module: None,
                kind: FileDependencyKind::External,
            });
        }
    }

    FileDependencyReport {
        file: file_display,
        module,
        imports: internal.len() + external.len(),
        internal,
        external,
    }
}

pub fn render_deps_file(rs_files: &[PathBuf], project_path: &Path, file: &Path, verbose: bool) {
    let report = analyze_file_deps(rs_files, project_path, file);
    render_file_dependency_report(&report, verbose);
}

pub fn render_deps_file_json(rs_files: &[PathBuf], project_path: &Path, file: &Path) {
    let report = analyze_file_deps(rs_files, project_path, file);
    println!("{}", serde_json::to_string(&report).unwrap());
}

pub fn render_file_dependency_report(report: &FileDependencyReport, verbose: bool) {
    crate_render::section_header("Dependencies");

    if verbose {
        crate_render::verbose_block(&[
            "Imports are raw use/mod targets parsed from the selected file.",
            "Internal imports resolve to known modules in the discovered crate files.",
            "External imports do not resolve to a known local module.",
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

    println!();
    println!("  {}", "internal imports:".bold());
    if report.internal.is_empty() {
        println!("  {}", "<none>".dimmed());
    } else {
        for dependency in &report.internal {
            println!(
                "  {} {} -> {}",
                "•".green(),
                dependency.raw,
                dependency.resolved_module.as_deref().unwrap_or("-")
            );
        }
    }

    println!();
    println!("  {}", "external imports:".bold());
    if report.external.is_empty() {
        println!("  {}", "<none>".dimmed());
    } else {
        for dependency in &report.external {
            println!("  {} {}", "•".yellow(), dependency.raw);
        }
    }
}

fn relative_path_string(project_path: &Path, path: &Path) -> String {
    path.strip_prefix(project_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn function_graph_name(file_module: &str, function_name: &str) -> String {
    if file_module == "main" || file_module == "lib" {
        function_name.to_string()
    } else {
        format!("{file_module}::{function_name}")
    }
}

/// Compute call cohesion: fraction of source functions calling another function in the same module.
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

/// Compute type cohesion: fraction of functions sharing signature type tokens within a module.
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
            .map(|f| extract_signature_types(&f.file, f.line_start, f.line_end))
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

/// Compute per-source-module cohesion metrics.
pub fn compute_cohesion(rs_files: &[PathBuf], project_path: &Path) -> Vec<ModuleCohesion> {
    let call_graph = flow::build_call_graph(rs_files, project_path);
    let symbols = ast_parser::parse_project(rs_files);

    let mut module_functions: HashMap<String, Vec<String>> = HashMap::new();
    let mut module_func_infos: HashMap<String, Vec<&ast_parser::FunctionInfo>> = HashMap::new();
    for file_syms in &symbols.files {
        let module_name = crate_render::module_name_from_path(&file_syms.path, project_path);
        for func in &file_syms.functions {
            module_functions
                .entry(module_name.clone())
                .or_default()
                .push(function_graph_name(&module_name, &func.name));
            module_func_infos
                .entry(module_name.clone())
                .or_default()
                .push(func);
        }
    }

    let call_cohesion_map = compute_call_cohesion(&module_functions, &call_graph);
    let type_cohesion_map = compute_type_cohesion(&module_func_infos);

    let mut results: Vec<ModuleCohesion> = module_func_infos
        .iter()
        .map(|(module, funcs)| {
            let call_c = call_cohesion_map.get(module).copied().unwrap_or(0.0);
            let type_c = type_cohesion_map.get(module).copied().unwrap_or(0.0);
            let combined = (0.6 * call_c + 0.4 * type_c).clamp(0.0, 1.0);
            ModuleCohesion {
                module: module.clone(),
                call_cohesion: call_c,
                type_cohesion: type_c,
                combined,
                function_count: funcs.len(),
            }
        })
        .collect();

    results.sort_by(|a, b| {
        a.combined
            .partial_cmp(&b.combined)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.module.cmp(&b.module))
    });
    results
}

/// Extract parameter/return type-like tokens from a function signature in a source file.
fn extract_signature_types(file: &Path, line_start: usize, line_end: usize) -> HashSet<String> {
    let content = match std::fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return HashSet::new(),
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = line_start.saturating_sub(1);
    let end = line_end.min(lines.len());
    let span_text: String = lines[start..end].join("\n");
    let signature_text = span_text
        .split_once('{')
        .map(|(signature, _)| signature)
        .unwrap_or(&span_text);

    let mut types = HashSet::new();
    for token in signature_text.split(|c: char| !c.is_alphanumeric() && c != '_') {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sibling_import_spelling_preserves_dependency_edges() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("cstat-sibling-import-{unique}"));
        let mut results = Vec::new();
        for source_root in ["src", "crates/member/src"] {
            let src = root.join(source_root);
            std::fs::create_dir_all(src.join("a")).unwrap();
            let parent = src.join("a/mod.rs");
            let importer = src.join("a/b.rs");
            let sibling = src.join("a/c.rs");
            std::fs::write(&parent, "mod b;\nmod c;\n").unwrap();
            std::fs::write(&sibling, "pub struct Thing;\n").unwrap();

            let unscored = [parent.clone(), importer.clone()];
            let scored = [parent, importer.clone(), sibling.clone()];
            let from = crate_render::module_name_from_path(&importer, &root);
            let to = crate_render::module_name_from_path(&sibling, &root);
            for path in [
                "super::c::Thing",
                "crate::a::c::Thing",
                "super::c",
                "crate::a::c",
            ] {
                std::fs::write(&importer, format!("use {path};\n")).unwrap();
                let targets = |files: &[PathBuf]| {
                    let edges = analyze_deps(files, &root)
                        .edges
                        .into_iter()
                        .filter_map(|edge| (edge.from == from).then_some(edge.to))
                        .collect::<Vec<_>>();
                    let report = analyze_file_deps(files, &root, &importer);
                    let imports: Vec<_> = report
                        .internal
                        .into_iter()
                        .filter_map(|dependency| dependency.resolved_module)
                        .collect();
                    assert_eq!(edges, imports, "{source_root}: {path}");
                    edges
                };
                results.push((
                    source_root,
                    path,
                    targets(&unscored),
                    targets(&scored),
                    to.clone(),
                ));
            }
        }
        std::fs::remove_dir_all(root).unwrap();

        for (source_root, path, unscored, scored, to) in results {
            assert!(
                unscored.is_empty(),
                "{source_root}: {path} resolved to {unscored:?}"
            );
            assert_eq!(scored, [to], "{source_root}: {path}");
        }
    }
}
