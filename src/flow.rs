use colored::Colorize;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use syn::visit::Visit;
use syn::{File as SynFile, ImplItem, Item};

use crate::render;

/// A call edge: caller function invokes callee function.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct CallEdge {
    pub caller: String,
    pub callee: String,
}

/// Kind of root used when tracing call paths.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryPointKind {
    Main,
    Test,
    Bench,
}

impl EntryPointKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntryPointKind::Main => "main",
            EntryPointKind::Test => "test",
            EntryPointKind::Bench => "bench",
        }
    }
}

/// Result of call graph analysis.
pub struct CallGraph {
    /// All known function names (qualified: Module::method or bare function).
    pub functions: Vec<String>,
    /// Edges: caller -> list of callees.
    pub adjacency: HashMap<String, Vec<String>>,
    /// Entry points (main, tests, and benchmarks).
    pub entry_points: Vec<String>,
    /// Entry point classification keyed by function name.
    pub entry_point_kinds: HashMap<String, EntryPointKind>,
    /// Edges that form cycles.
    pub cycle_edges: HashSet<(String, String)>,
}

/// AST visitor that extracts call sites from function bodies.
struct CallSiteExtractor {
    /// Current function being visited (qualified name).
    current_fn: String,
    /// Collected call edges.
    edges: Vec<CallEdge>,
}

impl<'ast> Visit<'ast> for CallSiteExtractor {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        // Extract the callee name from the call expression
        if let Some(name) = expr_to_callee_name(&node.func) {
            self.edges.push(CallEdge {
                caller: self.current_fn.clone(),
                callee: name,
            });
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        // Method calls: obj.method(args) — we record just the method name
        let name = node.method.to_string();
        self.edges.push(CallEdge {
            caller: self.current_fn.clone(),
            callee: name,
        });
        syn::visit::visit_expr_method_call(self, node);
    }
}

/// Try to extract a readable callee name from a call expression.
fn expr_to_callee_name(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Path(ep) => {
            let segments: Vec<String> = ep
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            Some(segments.join("::"))
        }
        _ => None,
    }
}

/// Extract all function definitions and their call sites from a single file.
fn extract_calls_from_file(
    path: &Path,
    project_path: &Path,
) -> (Vec<(String, Option<EntryPointKind>)>, Vec<CallEdge>) {
    // Returns: (list of (fn_name, entry kind), list of call edges)
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return (vec![], vec![]),
    };

    let ast: SynFile = match syn::parse_file(&content) {
        Ok(f) => f,
        Err(_) => return (vec![], vec![]),
    };

    let module = render::module_name_from_path(path, project_path);
    let base_modules: Vec<String> = if module == "main" || module == "lib" {
        Vec::new()
    } else {
        module.split("::").map(|s| s.to_string()).collect()
    };
    let mut functions: Vec<(String, Option<EntryPointKind>)> = Vec::new();
    let mut all_edges: Vec<CallEdge> = Vec::new();

    extract_items(
        &ast.items,
        path,
        project_path,
        &base_modules,
        &mut functions,
        &mut all_edges,
    );

    (functions, all_edges)
}

fn extract_items(
    items: &[Item],
    path: &Path,
    project_path: &Path,
    module_parts: &[String],
    functions: &mut Vec<(String, Option<EntryPointKind>)>,
    all_edges: &mut Vec<CallEdge>,
) {
    for item in items {
        match item {
            Item::Fn(func) => {
                let name = func.sig.ident.to_string();
                let qualified = qualify_name(module_parts, &name);
                let entry_kind = classify_entry_point(&name, &func.attrs, path, project_path);
                functions.push((qualified.clone(), entry_kind));

                let mut extractor = CallSiteExtractor {
                    current_fn: qualified,
                    edges: Vec::new(),
                };
                extractor.visit_block(&func.block);
                all_edges.extend(extractor.edges);
            }
            Item::Impl(imp) => {
                let type_name = qualify_name(module_parts, &type_to_short_string(&imp.self_ty));
                for impl_item in &imp.items {
                    if let ImplItem::Fn(method) = impl_item {
                        let method_name = method.sig.ident.to_string();
                        let qualified = format!("{}::{}", type_name, method_name);
                        let entry_kind =
                            classify_entry_point(&method_name, &method.attrs, path, project_path);
                        functions.push((qualified.clone(), entry_kind));

                        let mut extractor = CallSiteExtractor {
                            current_fn: qualified,
                            edges: Vec::new(),
                        };
                        extractor.visit_block(&method.block);
                        all_edges.extend(extractor.edges);
                    }
                }
            }
            Item::Mod(module) => {
                if let Some((_, nested_items)) = &module.content {
                    let mut nested_parts = module_parts.to_vec();
                    nested_parts.push(module.ident.to_string());
                    extract_items(
                        nested_items,
                        path,
                        project_path,
                        &nested_parts,
                        functions,
                        all_edges,
                    );
                }
            }
            _ => {}
        }
    }
}

fn qualify_name(module_parts: &[String], name: &str) -> String {
    if module_parts.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", module_parts.join("::"), name)
    }
}

fn classify_entry_point(
    name: &str,
    attrs: &[syn::Attribute],
    path: &Path,
    project_path: &Path,
) -> Option<EntryPointKind> {
    if has_attr_named(attrs, "test") {
        Some(EntryPointKind::Test)
    } else if has_attr_named(attrs, "bench") || is_under_dir(path, project_path, "benches") {
        Some(EntryPointKind::Bench)
    } else if name == "main" {
        Some(EntryPointKind::Main)
    } else {
        None
    }
}

fn has_attr_named(attrs: &[syn::Attribute], wanted: &str) -> bool {
    attrs.iter().any(|a| {
        a.path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == wanted)
    })
}

fn is_under_dir(path: &Path, project_path: &Path, dir_name: &str) -> bool {
    let rel = path.strip_prefix(project_path).unwrap_or(path);
    rel.components()
        .any(|component| component.as_os_str() == dir_name)
}

/// Convert a Type to a short string.
fn type_to_short_string(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(tp) => tp
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        _ => "?".to_string(),
    }
}

/// Resolve a callee name against known function names.
/// Tries exact match first, then caller-local sibling match, then suffix match.
fn resolve_callee(callee: &str, caller: &str, known: &HashMap<String, usize>) -> Option<String> {
    if known.contains_key(callee) {
        return Some(callee.to_string());
    }

    if !callee.contains("::") {
        if let Some((caller_parent, _)) = caller.rsplit_once("::") {
            let sibling = format!("{caller_parent}::{callee}");
            if known.contains_key(&sibling) {
                return Some(sibling);
            }
        }
    }

    let suffix = format!("::{}", callee);
    let mut matches: Vec<&String> = known.keys().filter(|k| k.ends_with(&suffix)).collect();
    if matches.len() == 1 {
        return Some(matches[0].clone());
    }

    if callee.contains("::") {
        matches = known.keys().filter(|k| k.ends_with(callee)).collect();
        if matches.len() == 1 {
            return Some(matches[0].clone());
        }
    }
    None
}

/// Detect cycles in the call graph using DFS. Returns the set of back-edges.
fn detect_cycles(adjacency: &HashMap<String, Vec<String>>) -> HashSet<(String, String)> {
    let mut cycle_edges = HashSet::new();
    let mut visited = HashSet::new();
    let mut on_stack = HashSet::new();

    for node in adjacency.keys() {
        if !visited.contains(node) {
            dfs_cycle(
                node,
                adjacency,
                &mut visited,
                &mut on_stack,
                &mut cycle_edges,
            );
        }
    }
    cycle_edges
}

fn dfs_cycle(
    node: &str,
    adjacency: &HashMap<String, Vec<String>>,
    visited: &mut HashSet<String>,
    on_stack: &mut HashSet<String>,
    cycle_edges: &mut HashSet<(String, String)>,
) {
    visited.insert(node.to_string());
    on_stack.insert(node.to_string());

    if let Some(callees) = adjacency.get(node) {
        for callee in callees {
            if on_stack.contains(callee.as_str()) {
                cycle_edges.insert((node.to_string(), callee.clone()));
            } else if !visited.contains(callee.as_str()) {
                dfs_cycle(callee, adjacency, visited, on_stack, cycle_edges);
            }
        }
    }

    on_stack.remove(node);
}

/// Build the call graph from all project files.
pub fn build_call_graph(rs_files: &[PathBuf], project_path: &Path) -> CallGraph {
    let mut all_functions: Vec<(String, Option<EntryPointKind>)> = Vec::new();
    let mut all_edges: Vec<CallEdge> = Vec::new();

    for file in rs_files {
        let (funcs, edges) = extract_calls_from_file(file, project_path);
        all_functions.extend(funcs);
        all_edges.extend(edges);
    }

    // Build a set of known function names (with index for quick lookup)
    let known: HashMap<String, usize> = all_functions
        .iter()
        .enumerate()
        .map(|(i, (name, _))| (name.clone(), i))
        .collect();

    let functions: Vec<String> = all_functions.iter().map(|(n, _)| n.clone()).collect();
    let entry_point_kinds: HashMap<String, EntryPointKind> = all_functions
        .iter()
        .filter_map(|(name, kind)| kind.clone().map(|k| (name.clone(), k)))
        .collect();
    let entry_points: Vec<String> = all_functions
        .iter()
        .filter(|(name, _)| entry_point_kinds.contains_key(name))
        .map(|(n, _)| n.clone())
        .collect();

    // Resolve edges against known functions
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for edge in &all_edges {
        if let Some(resolved) = resolve_callee(&edge.callee, &edge.caller, &known) {
            if resolved != edge.caller {
                adjacency
                    .entry(edge.caller.clone())
                    .or_default()
                    .push(resolved);
            }
        }
    }

    // Deduplicate adjacency lists while preserving order
    for callees in adjacency.values_mut() {
        let mut seen = HashSet::new();
        callees.retain(|c| seen.insert(c.clone()));
    }

    let cycle_edges = detect_cycles(&adjacency);

    CallGraph {
        functions,
        adjacency,
        entry_points,
        entry_point_kinds,
        cycle_edges,
    }
}

/// Return every function reachable from the graph's entry points.
pub fn reachable_from_entries(call_graph: &CallGraph) -> HashSet<String> {
    let mut reachable = HashSet::new();
    let mut queue: Vec<String> = call_graph.entry_points.clone();
    while let Some(node) = queue.pop() {
        if !reachable.insert(node.clone()) {
            continue;
        }
        if let Some(callees) = call_graph.adjacency.get(&node) {
            for callee in callees {
                if !reachable.contains(callee) {
                    queue.push(callee.clone());
                }
            }
        }
    }
    reachable
}

/// Return entry points matching a specific kind.
pub fn entry_points_by_kind(call_graph: &CallGraph, kind: EntryPointKind) -> Vec<String> {
    call_graph
        .entry_points
        .iter()
        .filter(|entry| call_graph.entry_point_kinds.get(*entry) == Some(&kind))
        .cloned()
        .collect()
}

/// Render the call graph as a tree rooted at entry points.
fn render_call_tree(graph: &CallGraph, verbose: bool) {
    println!(
        "\n{}",
        "── Call Flow ─────────────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Static call graph rendered as a tree rooted at entry points.",
            "Entry points are main() and #[test] functions.",
            "Each indented line is a callee of the function above it.",
            "├── / └── = tree branch connectors. ─┬ = node has children below.",
            "⟲ (red) = cycle detected — this call creates a recursive loop.",
            "… (dimmed) = already visited in this tree — prevents infinite expansion.",
            "⋯ (depth limit) = tree truncated at maximum display depth (12 levels).",
            "Yellow-highlighted names = functions that themselves have callees.",
        ]);
        render::guide_ref("flow");
    }

    if graph.entry_points.is_empty() {
        println!("  {}", "No entry points found.".dimmed());
        return;
    }

    // Render from each entry point
    for entry in &graph.entry_points {
        println!();
        println!("  {}", entry.bold().bright_white());
        let mut visited = HashSet::new();
        visited.insert(entry.clone());
        render_subtree(graph, entry, "  ", &mut visited, 0);
    }
}

/// Recursively render a subtree of the call graph.
fn render_subtree(
    graph: &CallGraph,
    node: &str,
    prefix: &str,
    visited: &mut HashSet<String>,
    depth: usize,
) {
    const MAX_DEPTH: usize = 12;

    let callees = match graph.adjacency.get(node) {
        Some(c) if !c.is_empty() => c,
        _ => return,
    };

    let count = callees.len();
    for (i, callee) in callees.iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last { "    " } else { "│   " };

        let is_cycle = graph
            .cycle_edges
            .contains(&(node.to_string(), callee.clone()));
        let is_revisit = visited.contains(callee);

        if is_cycle {
            println!(
                "{}{}{}",
                prefix,
                connector,
                format!("{} ⟲ cycle", callee).red().bold(),
            );
        } else if is_revisit {
            println!(
                "{}{}{}",
                prefix,
                connector,
                format!("{} …", callee).dimmed(),
            );
        } else if depth >= MAX_DEPTH {
            println!(
                "{}{}{}",
                prefix,
                connector,
                format!("{} ⋯ (depth limit)", callee).dimmed(),
            );
        } else {
            let has_children = graph
                .adjacency
                .get(callee.as_str())
                .map_or(false, |c| !c.is_empty());

            if has_children {
                println!(
                    "{}{}{}",
                    prefix,
                    connector.replace("── ", "─┬ "),
                    callee.bright_yellow(),
                );
            } else {
                println!("{}{}{}", prefix, connector, callee);
            }

            visited.insert(callee.clone());
            let new_prefix = format!("{}{}", prefix, child_prefix);
            render_subtree(graph, callee, &new_prefix, visited, depth + 1);
        }
    }
}

/// Render call graph statistics.
fn render_stats(graph: &CallGraph, verbose: bool) {
    println!(
        "\n{}",
        "── Call Graph Stats ──────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Summary statistics for the call graph.",
            "Functions = total discovered. Call edges = total caller→callee links.",
            "Entry points = main() and #[test] functions. Cycles (red) = back-edges detected.",
            "  Cycles = none (green) means the call graph is a DAG.",
            "Fan-out (top callers): functions that call the most other functions.",
            "  High fan-out = orchestrator or god function that coordinates many pieces.",
            "  Bar (blue) length proportional to call count.",
            "Hot paths (longest call chains): deepest call chain from each entry point.",
            "  Depth = number of function hops from entry to deepest leaf.",
            "  Color: red = depth > 6, yellow = depth > 3, white = shallow.",
        ]);
    }

    let total_fns = graph.functions.len();
    let total_edges: usize = graph.adjacency.values().map(|v| v.len()).sum();
    let cycle_count = graph.cycle_edges.len();

    println!();
    println!("  Functions:    {}", format!("{}", total_fns).bold());
    println!("  Call edges:   {}", format!("{}", total_edges).bold());
    println!(
        "  Entry points: {}",
        format!("{}", graph.entry_points.len()).bold()
    );

    if cycle_count > 0 {
        println!(
            "  Cycles:       {}",
            format!("{}", cycle_count).red().bold()
        );
    } else {
        println!("  Cycles:       {}", "none".green());
    }

    // Fan-out: functions that call the most others
    let mut fan_out: Vec<(&String, usize)> =
        graph.adjacency.iter().map(|(k, v)| (k, v.len())).collect();
    fan_out.sort_by(|a, b| b.1.cmp(&a.1));

    if !fan_out.is_empty() {
        println!();
        println!("  {} (top callers):", "Fan-out".bold().underline());
        for (name, count) in fan_out.iter().take(8) {
            let bar_len = (*count).min(30);
            let bar = "█".repeat(bar_len);
            println!("    {:<40} {} {}", name.dimmed(), bar.bright_blue(), count,);
        }
    }

    // Deepest call chains
    println!();
    println!(
        "  {} (longest call chains):",
        "Hot paths".bold().underline()
    );
    for entry in &graph.entry_points {
        let depth = measure_max_depth(graph, entry, &mut HashSet::new());
        let indicator = if depth > 6 {
            format!("depth {}", depth).red().bold().to_string()
        } else if depth > 3 {
            format!("depth {}", depth).yellow().to_string()
        } else {
            format!("depth {}", depth).to_string()
        };
        println!("    {} → {}", entry.bright_white(), indicator);
    }

    println!();
}

/// Measure the maximum call depth from a given node.
fn measure_max_depth(graph: &CallGraph, node: &str, visited: &mut HashSet<String>) -> usize {
    if visited.contains(node) {
        return 0;
    }
    visited.insert(node.to_string());

    let max_child = graph
        .adjacency
        .get(node)
        .map(|callees| {
            callees
                .iter()
                .map(|c| measure_max_depth(graph, c, visited))
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);

    visited.remove(node);
    1 + max_child
}

/// Public entry point: render the flow subcommand.
pub fn render_flow(rs_files: &[PathBuf], project_path: &Path, verbose: bool) {
    let graph = build_call_graph(rs_files, project_path);

    if graph.functions.is_empty() {
        println!("{}", "No functions found to analyze.".yellow());
        return;
    }

    render_call_tree(&graph, verbose);
    render_stats(&graph, verbose);
}

#[derive(Serialize)]
struct CallEdgeJson {
    caller: String,
    callee: String,
}

#[derive(Serialize)]
struct FanOutJson {
    function: String,
    call_count: usize,
}

#[derive(Serialize)]
struct EntryDepthJson {
    entry_point: String,
    max_depth: usize,
}

#[derive(Serialize)]
struct FlowJson {
    cstat_version: String,
    functions: Vec<String>,
    call_edges: Vec<CallEdgeJson>,
    entry_points: Vec<String>,
    cycles: Vec<CallEdgeJson>,
    max_depth: usize,
    top_callers: Vec<FanOutJson>,
    entry_depths: Vec<EntryDepthJson>,
    cold_functions: Vec<String>,
}

/// Render flow analysis as JSON.
pub fn render_flow_json(rs_files: &[PathBuf], project_path: &Path) {
    let graph = build_call_graph(rs_files, project_path);

    let mut call_edges = Vec::new();
    for (caller, callees) in &graph.adjacency {
        for callee in callees {
            call_edges.push(CallEdgeJson {
                caller: caller.clone(),
                callee: callee.clone(),
            });
        }
    }

    let cycles: Vec<CallEdgeJson> = graph
        .cycle_edges
        .iter()
        .map(|(a, b)| CallEdgeJson {
            caller: a.clone(),
            callee: b.clone(),
        })
        .collect();

    let mut fan_out: Vec<(&String, usize)> =
        graph.adjacency.iter().map(|(k, v)| (k, v.len())).collect();
    fan_out.sort_by(|a, b| b.1.cmp(&a.1));
    let top_callers: Vec<FanOutJson> = fan_out
        .iter()
        .take(20)
        .map(|(f, c)| FanOutJson {
            function: (*f).clone(),
            call_count: *c,
        })
        .collect();

    let entry_depths: Vec<EntryDepthJson> = graph
        .entry_points
        .iter()
        .map(|entry| {
            let depth = measure_max_depth(&graph, entry, &mut HashSet::new());
            EntryDepthJson {
                entry_point: entry.clone(),
                max_depth: depth,
            }
        })
        .collect();

    let max_depth = entry_depths.iter().map(|e| e.max_depth).max().unwrap_or(0);

    // Cold functions
    let reachable = {
        let mut reachable = HashSet::new();
        let mut queue: Vec<String> = graph.entry_points.clone();
        while let Some(node) = queue.pop() {
            if !reachable.insert(node.clone()) {
                continue;
            }
            if let Some(callees) = graph.adjacency.get(&node) {
                for callee in callees {
                    if !reachable.contains(callee) {
                        queue.push(callee.clone());
                    }
                }
            }
        }
        reachable
    };
    let cold_functions: Vec<String> = graph
        .functions
        .iter()
        .filter(|f| !reachable.contains(*f))
        .cloned()
        .collect();

    let output = FlowJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        functions: graph.functions,
        call_edges,
        entry_points: graph.entry_points,
        cycles,
        max_depth,
        top_callers,
        entry_depths,
        cold_functions,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("cstat-flow-{name}-{unique}"));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        root
    }

    #[test]
    fn resolves_unqualified_sibling_call_when_suffix_is_ambiguous() {
        let root = temp_project("sibling-run");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
mod cli {
    pub fn run_from_args<I>(args: I) -> std::process::ExitCode
    where
        I: IntoIterator<Item = String>,
    {
        install_signal_handlers();
        match run(args) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("mvp-one-node-chat: {error}");
                std::process::ExitCode::from(1)
            }
        }
    }

    fn install_signal_handlers() {}

    fn run<I>(_args: I) -> Result<(), String>
    where
        I: IntoIterator<Item = String>,
    {
        Ok(())
    }
}

mod worker {
    fn run() {}
}

fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        let callees = graph
            .adjacency
            .get("cli::run_from_args")
            .expect("run_from_args should have outgoing edges");

        assert!(
            callees.contains(&"cli::run".to_string()),
            "callee list did not include sibling run(): {callees:?}"
        );

        fs::remove_dir_all(root).unwrap();
    }
}
