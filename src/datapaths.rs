use colored::Colorize;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::path::PathBuf;

use crate::flow::{self, build_call_graph, CallGraph, EntryPointKind};
use crate::{ast_parser, file_metrics, render};

#[derive(Debug, Clone, Serialize)]
pub struct TestReachabilityEntry {
    pub entry: String,
    pub kind: String,
    pub reachable_functions: usize,
    pub unique_call_steps: usize,
    pub max_call_depth: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionReachabilityCount {
    pub function: String,
    pub reachable_from_count: usize,
    pub reachable_from_entries: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EdgeReachabilityCount {
    pub caller: String,
    pub callee: String,
    pub reachable_from_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestReachabilityAnalysis {
    pub total_functions: usize,
    pub test_benchmark_entry_count: usize,
    pub reachable_function_count: usize,
    pub unreachable_function_count: usize,
    pub entries: Vec<TestReachabilityEntry>,
    pub function_reachability_counts: Vec<FunctionReachabilityCount>,
    pub edge_reachability_counts: Vec<EdgeReachabilityCount>,
    pub unreachable_functions: Vec<String>,
    pub analysis_note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileStaticReachabilityEntry {
    pub entry: String,
    pub kind: String,
    pub reachable_functions: usize,
    pub unique_call_steps: usize,
    pub max_call_depth: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileStaticReachabilityEdge {
    pub caller: String,
    pub callee: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileStaticReachabilityReport {
    pub file: String,
    pub test_benchmark_entry_count: usize,
    pub production_function_count: usize,
    pub reachable_function_count: usize,
    pub unreachable_function_count: usize,
    pub local_edge_count: usize,
    pub reachable_local_edge_count: usize,
    pub entries: Vec<FileStaticReachabilityEntry>,
    pub reachable_functions: Vec<String>,
    pub unreachable_functions: Vec<String>,
    pub reachable_edges: Vec<FileStaticReachabilityEdge>,
    pub wrapper_entrypoints: Vec<String>,
}

pub fn analyze_test_reachability(
    rs_files: &[PathBuf],
    project_path: &Path,
) -> TestReachabilityAnalysis {
    let graph = build_call_graph(rs_files, project_path);
    analyze_graph_test_reachability(&graph)
}

pub fn analyze_graph_test_reachability(graph: &CallGraph) -> TestReachabilityAnalysis {
    let mut entry_roots = flow::entry_points_by_kind(graph, EntryPointKind::Test);
    entry_roots.extend(flow::entry_points_by_kind(graph, EntryPointKind::Bench));
    entry_roots.sort();
    entry_roots.dedup();

    let mut function_to_entries: HashMap<String, Vec<String>> = HashMap::new();
    let mut edge_to_entries: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut entries = Vec::new();

    for entry in &entry_roots {
        let (reachable, edges) = reachable_subgraph(graph, entry);
        for function in &reachable {
            function_to_entries
                .entry(function.clone())
                .or_default()
                .push(entry.clone());
        }
        for edge in &edges {
            edge_to_entries
                .entry(edge.clone())
                .or_default()
                .push(entry.clone());
        }

        let kind = graph
            .entry_point_kinds
            .get(entry)
            .map(|k| k.as_str().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        entries.push(TestReachabilityEntry {
            entry: entry.clone(),
            kind,
            reachable_functions: reachable.len(),
            unique_call_steps: edges.len(),
            max_call_depth: max_depth_from(graph, entry, &mut HashSet::new()),
        });
    }

    entries.sort_by(|a, b| a.entry.cmp(&b.entry));

    let mut function_reachability_counts: Vec<FunctionReachabilityCount> = graph
        .functions
        .iter()
        .map(|function| {
            let mut reachable_from_entries =
                function_to_entries.remove(function).unwrap_or_default();
            reachable_from_entries.sort();
            reachable_from_entries.dedup();
            FunctionReachabilityCount {
                function: function.clone(),
                reachable_from_count: reachable_from_entries.len(),
                reachable_from_entries,
            }
        })
        .collect();
    function_reachability_counts.sort_by(|a, b| {
        b.reachable_from_count
            .cmp(&a.reachable_from_count)
            .then(a.function.cmp(&b.function))
    });

    let mut edge_reachability_counts: Vec<EdgeReachabilityCount> = edge_to_entries
        .into_iter()
        .map(|((caller, callee), mut reachable_from_entries)| {
            reachable_from_entries.sort();
            reachable_from_entries.dedup();
            EdgeReachabilityCount {
                caller,
                callee,
                reachable_from_count: reachable_from_entries.len(),
            }
        })
        .collect();
    edge_reachability_counts.sort_by(|a, b| {
        b.reachable_from_count
            .cmp(&a.reachable_from_count)
            .then(a.caller.cmp(&b.caller))
            .then(a.callee.cmp(&b.callee))
    });

    let mut unreachable_functions: Vec<String> = function_reachability_counts
        .iter()
        .filter(|count| count.reachable_from_count == 0)
        .map(|count| count.function.clone())
        .collect();
    unreachable_functions.sort();

    TestReachabilityAnalysis {
        total_functions: graph.functions.len(),
        test_benchmark_entry_count: entry_roots.len(),
        reachable_function_count: graph.functions.len().saturating_sub(unreachable_functions.len()),
        unreachable_function_count: unreachable_functions.len(),
        entries,
        function_reachability_counts,
        edge_reachability_counts,
        unreachable_functions,
        analysis_note: "Static test/benchmark reachability. Counts are how many test or benchmark entry roots can reach a function/edge, not runtime coverage or hit counts.".to_string(),
    }
}

pub fn analyze_file_static_reachability(
    graph: &CallGraph,
    file_functions: &file_metrics::FileFunctionIndex,
    file_display_path: &str,
) -> FileStaticReachabilityReport {
    let selected: HashSet<String> = file_functions
        .functions
        .iter()
        .map(|function| function.graph_name.clone())
        .collect();
    let production: HashSet<String> = file_functions
        .functions
        .iter()
        .filter(|function| {
            !function.is_test
                && !function.is_bench
                && !function.is_test_support
                && !function.is_binary_wrapper_main
        })
        .map(|function| function.graph_name.clone())
        .collect();
    let mut wrapper_entrypoints: Vec<String> = file_functions
        .functions
        .iter()
        .filter(|function| function.is_binary_wrapper_main)
        .map(|function| function.display_name.clone())
        .collect();
    wrapper_entrypoints.sort();

    let mut roots = flow::entry_points_by_kind(graph, EntryPointKind::Test);
    roots.extend(flow::entry_points_by_kind(graph, EntryPointKind::Bench));
    roots.sort();
    roots.dedup();

    let mut reachable_functions_set: HashSet<String> = HashSet::new();
    let mut reachable_edges_set: HashSet<(String, String)> = HashSet::new();
    let mut entries = Vec::new();

    for root in &roots {
        let (reachable, edges) = reachable_subgraph(graph, root);
        let root_reachable_functions = reachable
            .iter()
            .filter(|function| production.contains(*function))
            .count();
        let root_reachable_edges = edges
            .iter()
            .filter(|(caller, callee)| selected.contains(caller) && selected.contains(callee))
            .count();

        reachable_functions_set.extend(
            reachable
                .iter()
                .filter(|function| production.contains(*function))
                .cloned(),
        );
        reachable_edges_set.extend(
            edges
                .iter()
                .filter(|(caller, callee)| selected.contains(caller) && selected.contains(callee))
                .cloned(),
        );

        let kind = graph
            .entry_point_kinds
            .get(root)
            .map(|kind| kind.as_str().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        entries.push(FileStaticReachabilityEntry {
            entry: root.clone(),
            kind,
            reachable_functions: root_reachable_functions,
            unique_call_steps: root_reachable_edges,
            max_call_depth: max_depth_from(graph, root, &mut HashSet::new()),
        });
    }

    entries.sort_by(|a, b| a.entry.cmp(&b.entry));

    let mut reachable_functions: Vec<String> = reachable_functions_set.into_iter().collect();
    reachable_functions.sort();
    let mut unreachable_functions: Vec<String> = production
        .iter()
        .filter(|function| !reachable_functions.contains(function))
        .cloned()
        .collect();
    unreachable_functions.sort();

    let local_edge_count = graph
        .adjacency
        .iter()
        .map(|(caller, callees)| {
            callees
                .iter()
                .filter(|callee| selected.contains(caller) && selected.contains(*callee))
                .count()
        })
        .sum();

    let mut reachable_edges: Vec<FileStaticReachabilityEdge> = reachable_edges_set
        .into_iter()
        .map(|(caller, callee)| FileStaticReachabilityEdge { caller, callee })
        .collect();
    reachable_edges.sort_by(|a, b| a.caller.cmp(&b.caller).then(a.callee.cmp(&b.callee)));

    FileStaticReachabilityReport {
        file: file_display_path.to_string(),
        test_benchmark_entry_count: entries.len(),
        production_function_count: production.len(),
        reachable_function_count: reachable_functions.len(),
        unreachable_function_count: unreachable_functions.len(),
        local_edge_count,
        reachable_local_edge_count: reachable_edges.len(),
        entries,
        reachable_functions,
        unreachable_functions,
        reachable_edges,
        wrapper_entrypoints,
    }
}

fn reachable_subgraph(
    graph: &CallGraph,
    entry: &str,
) -> (HashSet<String>, HashSet<(String, String)>) {
    let mut reachable = HashSet::new();
    let mut edges = HashSet::new();
    let mut stack = vec![entry.to_string()];

    while let Some(node) = stack.pop() {
        if !reachable.insert(node.clone()) {
            continue;
        }
        if let Some(callees) = graph.adjacency.get(&node) {
            for callee in callees {
                edges.insert((node.clone(), callee.clone()));
                if !reachable.contains(callee) {
                    stack.push(callee.clone());
                }
            }
        }
    }

    (reachable, edges)
}

fn max_depth_from(graph: &CallGraph, node: &str, visited: &mut HashSet<String>) -> usize {
    if !visited.insert(node.to_string()) {
        return 0;
    }

    let depth = graph
        .adjacency
        .get(node)
        .map(|callees| {
            callees
                .iter()
                .map(|callee| 1 + max_depth_from(graph, callee, visited))
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);

    visited.remove(node);
    depth
}

pub fn render_test_reachability_report(report: &TestReachabilityAnalysis, verbose: bool) {
    render::section_header("Static test/benchmark reachability");

    if verbose {
        render::verbose_block(&[
            "Each entry root is a #[test], #[bench], or function in benches/.",
            "Reachability is computed from the static call graph; it is not runtime coverage.",
            "Reachability count is how many test/bench roots can reach a function or edge.",
        ]);
    }

    let reachability_pct = if report.total_functions == 0 {
        0.0
    } else {
        report.reachable_function_count as f64 / report.total_functions as f64 * 100.0
    };

    println!(
        "  {} {}  {} {} / {} ({:.1}%)  {} {}",
        "test/bench roots".cyan(),
        report.test_benchmark_entry_count.to_string().bold(),
        "statically reachable".cyan(),
        report.reachable_function_count.to_string().bold(),
        report.total_functions,
        reachability_pct,
        "unreachable".cyan(),
        report.unreachable_function_count.to_string().bold(),
    );

    if report.entries.is_empty() {
        println!("  {}", "No test or benchmark entry roots found.".yellow());
    } else {
        println!();
        println!(
            "  {:<48} {:<7} {:>9} {:>7} {:>8}",
            "entry root".bold(),
            "kind".bold(),
            "functions".bold(),
            "steps".bold(),
            "depth".bold(),
        );
        for entry in &report.entries {
            println!(
                "  {:<48} {:<7} {:>9} {:>7} {:>8}",
                truncate(&entry.entry, 48),
                entry.kind,
                entry.reachable_functions,
                entry.unique_call_steps,
                entry.max_call_depth,
            );
        }
    }

    let reachable_functions: Vec<&FunctionReachabilityCount> = report
        .function_reachability_counts
        .iter()
        .filter(|count| count.reachable_from_count > 0)
        .take(20)
        .collect();
    if !reachable_functions.is_empty() {
        println!();
        println!("  {}", "Functions reachable from most entry roots:".bold());
        for count in reachable_functions {
            println!(
                "  {:>3}×  {}",
                count.reachable_from_count,
                truncate(&count.function, 70)
            );
        }
    }

    if !report.unreachable_functions.is_empty() {
        println!();
        println!(
            "  {}",
            "Not statically reachable from tests/benches:"
                .bold()
                .yellow()
        );
        for function in report.unreachable_functions.iter().take(20) {
            println!("  {} {}", "•".yellow(), function);
        }
        if report.unreachable_functions.len() > 20 {
            println!("  … +{} more", report.unreachable_functions.len() - 20);
        }
    }
}

pub fn render_test_reachability(rs_files: &[PathBuf], project_path: &Path, verbose: bool) {
    let report = analyze_test_reachability(rs_files, project_path);
    render_test_reachability_report(&report, verbose);
}

pub fn render_test_reachability_json(rs_files: &[PathBuf], project_path: &Path) {
    let report = analyze_test_reachability(rs_files, project_path);
    println!("{}", serde_json::to_string(&report).unwrap());
}

pub fn render_test_reachability_file(
    rs_files: &[PathBuf],
    project_path: &Path,
    file: &Path,
    verbose: bool,
) {
    let report = analyze_file_static_reachability_from_files(rs_files, project_path, file);
    render_file_static_reachability_report(&report, verbose);
}

pub fn render_test_reachability_file_json(rs_files: &[PathBuf], project_path: &Path, file: &Path) {
    let report = analyze_file_static_reachability_from_files(rs_files, project_path, file);
    println!("{}", serde_json::to_string(&report).unwrap());
}

pub fn analyze_file_static_reachability_from_files(
    rs_files: &[PathBuf],
    project_path: &Path,
    file: &Path,
) -> FileStaticReachabilityReport {
    let symbols = ast_parser::parse_project(rs_files);
    let graph = build_call_graph(rs_files, project_path);
    let file_functions =
        file_metrics::selected_file_functions(&symbols, &graph, project_path, file);
    let file_display_path = relative_path_string(project_path, file);
    analyze_file_static_reachability(&graph, &file_functions, &file_display_path)
}

pub fn render_file_static_reachability_report(
    report: &FileStaticReachabilityReport,
    verbose: bool,
) {
    render::section_header("Static test/benchmark reachability");

    if verbose {
        render::verbose_block(&[
            "Roots are tests and benchmarks discovered anywhere in the project.",
            "Reachability is computed from the static call graph; it is not runtime coverage.",
            "Production counts exclude tests, test-support helpers, benches, and selected binary wrapper main().",
        ]);
    }

    let pct = if report.production_function_count == 0 {
        0.0
    } else {
        report.reachable_function_count as f64 / report.production_function_count as f64 * 100.0
    };
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

    let visible_entries: Vec<&FileStaticReachabilityEntry> = report
        .entries
        .iter()
        .filter(|entry| entry.reachable_functions > 0 || entry.unique_call_steps > 0)
        .collect();

    println!();
    println!(
        "  {:<48} {:<7} {:>9} {:>7} {:>8}",
        "entry root".bold(),
        "kind".bold(),
        "functions".bold(),
        "steps".bold(),
        "depth".bold(),
    );
    if report.entries.is_empty() {
        println!("  {}", "No project test or benchmark roots found.".yellow());
    } else if visible_entries.is_empty() {
        println!(
            "  {}",
            "No project test or benchmark roots reach selected production functions.".yellow()
        );
    } else {
        for entry in visible_entries {
            println!(
                "  {:<48} {:<7} {:>9} {:>7} {:>8}",
                truncate(&entry.entry, 48),
                entry.kind,
                entry.reachable_functions,
                entry.unique_call_steps,
                entry.max_call_depth,
            );
        }
    }

    println!();
    println!("  {}", "reachable functions:".bold());
    if report.reachable_functions.is_empty() {
        println!("  {}", "<none>".dimmed());
    } else {
        for function in &report.reachable_functions {
            println!("  {} {}", "•".green(), function);
        }
    }

    println!();
    println!("  {}", "not statically reachable:".bold().yellow());
    if report.unreachable_functions.is_empty() {
        println!("  {}", "<none>".dimmed());
    } else {
        for function in &report.unreachable_functions {
            println!("  {} {}", "•".yellow(), function);
        }
    }

    println!();
    println!("  {}", "reachable local edges:".bold());
    if report.reachable_edges.is_empty() {
        println!("  {}", "<none>".dimmed());
    } else {
        for edge in &report.reachable_edges {
            println!("  {} {} -> {}", "•".green(), edge.caller, edge.callee);
        }
    }
}

fn relative_path_string(project_path: &Path, path: &Path) -> String {
    path.strip_prefix(project_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
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
        let root = std::env::temp_dir().join(format!("cstat-{name}-{unique}"));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        root
    }

    #[test]
    fn static_test_reachability_counts_entry_roots_steps_and_frequency() {
        let root = temp_project("datapaths");
        let lib = root.join("src/lib.rs");
        fs::write(
            &lib,
            r#"
fn target() {
    helper();
}

fn helper() {}

mod tests {
    #[test]
    fn covers_target() {
        target();
    }
}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[lib], &root);
        let analysis = analyze_graph_test_reachability(&graph);

        assert_eq!(analysis.test_benchmark_entry_count, 1);
        assert_eq!(analysis.reachable_function_count, 3);
        assert_eq!(analysis.unreachable_function_count, 0);

        let entry = analysis
            .entries
            .iter()
            .find(|entry| entry.entry == "tests::covers_target")
            .unwrap();
        assert_eq!(entry.kind, "test");
        assert_eq!(entry.reachable_functions, 3);
        assert_eq!(entry.unique_call_steps, 2);
        assert_eq!(entry.max_call_depth, 2);

        let helper = analysis
            .function_reachability_counts
            .iter()
            .find(|count| count.function == "helper")
            .unwrap();
        assert_eq!(helper.reachable_from_count, 1);
        assert_eq!(
            helper.reachable_from_entries,
            vec!["tests::covers_target".to_string()]
        );

        fs::remove_dir_all(root).unwrap();
    }
}
