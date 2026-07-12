mod algorithms;

use colored::Colorize;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::deps;
use crate::flow;
use crate::render;
use algorithms::{
    betweenness_centrality, clustering_coefficient, graph_level_stats, pagerank, tarjan_scc,
    DiGraph, GraphStats,
};

// ── Full analysis result ───────────────────────────────────────────

struct NodeMetrics {
    betweenness: f64,
    pagerank: f64,
    clustering: f64,
}

struct GraphAnalysis {
    label: String,
    node_count: usize,
    edge_count: usize,
    sccs: Vec<Vec<String>>,
    node_metrics: HashMap<String, NodeMetrics>,
    stats: GraphStats,
}

fn analyze_graph(graph: &DiGraph, label: &str) -> GraphAnalysis {
    let sccs = tarjan_scc(graph);
    let bc = betweenness_centrality(graph);
    let pr = pagerank(graph, 0.85, 100);
    let cc = clustering_coefficient(graph);
    let stats = graph_level_stats(graph);

    let mut node_metrics: HashMap<String, NodeMetrics> = HashMap::new();
    for node in &graph.nodes {
        node_metrics.insert(
            node.clone(),
            NodeMetrics {
                betweenness: bc.get(node).copied().unwrap_or(0.0),
                pagerank: pr.get(node).copied().unwrap_or(0.0),
                clustering: cc.get(node).copied().unwrap_or(0.0),
            },
        );
    }

    GraphAnalysis {
        label: label.to_string(),
        node_count: graph.node_count(),
        edge_count: graph.edge_count(),
        sccs,
        node_metrics,
        stats,
    }
}

// ── Build DiGraph from call graph / dep graph ──────────────────────

fn digraph_from_call_graph(cg: &flow::CallGraph) -> DiGraph {
    let nodes = cg.functions.clone();
    let mut edges = Vec::new();
    for (caller, callees) in &cg.adjacency {
        for callee in callees {
            edges.push((caller.clone(), callee.clone()));
        }
    }
    DiGraph::from_edges(nodes, &edges)
}

fn digraph_from_dep_analysis(da: &deps::DepAnalysis) -> DiGraph {
    let nodes = da.modules.clone();
    let edges: Vec<(String, String)> = da
        .edges
        .iter()
        .map(|e| (e.from.clone(), e.to.clone()))
        .collect();
    DiGraph::from_edges(nodes, &edges)
}

// ── Rendering ──────────────────────────────────────────────────────

fn render_analysis(analysis: &GraphAnalysis, top_n: usize, verbose: bool) {
    let term_w = render::terminal_width();

    let header = format!("── {} ─", analysis.label);
    let padded = format!("{:─<width$}", header, width = 56);
    println!("\n{}", padded.bright_cyan().bold());

    render_stats_box(analysis, verbose);
    render_sccs(analysis, verbose);
    render_node_table(analysis, top_n, term_w, verbose);
}

fn render_stats_box(analysis: &GraphAnalysis, verbose: bool) {
    if verbose {
        render::verbose_block(&[
            "Graph-level statistics for the overall structure:",
            "  Nodes = number of functions (or modules for dependency graph).",
            "  Edges = total directed connections between nodes.",
            "  Density = edges / (nodes × (nodes - 1)). Ranges 0 to 1.",
            "    Low density = sparse graph; high density = many connections.",
            "  Components (weak) = weakly connected subgraphs (ignoring edge direction).",
            "    >1 component = some nodes are completely isolated from others.",
            "  Diameter = longest shortest path between any two reachable nodes.",
            "  Avg path length = mean shortest path distance across all reachable pairs.",
        ]);
        render::guide_ref("architecture");
    }

    println!();
    println!("  ┌──────────────────────────────────────┐");
    println!(
        "  │ {:<18} {:>17} │",
        "Nodes".cyan(),
        format!("{}", analysis.node_count).bold()
    );
    println!(
        "  │ {:<18} {:>17} │",
        "Edges".cyan(),
        format!("{}", analysis.edge_count).bold()
    );
    println!(
        "  │ {:<18} {:>17} │",
        "Density".cyan(),
        format!("{:.4}", analysis.stats.density).bold()
    );
    println!(
        "  │ {:<18} {:>17} │",
        "Components (weak)".cyan(),
        format!("{}", analysis.stats.component_count).bold()
    );
    if let Some(d) = analysis.stats.diameter {
        println!(
            "  │ {:<18} {:>17} │",
            "Diameter".cyan(),
            format!("{}", d).bold()
        );
    }
    if analysis.stats.avg_path_length > 0.0 {
        println!(
            "  │ {:<18} {:>17} │",
            "Avg path length".cyan(),
            format!("{:.2}", analysis.stats.avg_path_length).bold()
        );
    }
    println!("  └──────────────────────────────────────┘");
}

fn render_sccs(analysis: &GraphAnalysis, verbose: bool) {
    let cyclic_sccs: Vec<&Vec<String>> = analysis.sccs.iter().filter(|c| c.len() > 1).collect();

    println!("\n  {}", "Strongly Connected Components".bold().underline());

    if verbose {
        render::verbose_block(&[
            "SCCs found by Tarjan's algorithm: groups of mutually reachable nodes (cycles).",
            "Every node in an SCC can reach every other node in the same SCC.",
            "A DAG (directed acyclic graph) has no cyclic SCCs — all components are singletons.",
            "Cyclic clusters indicate circular dependencies that may complicate refactoring.",
        ]);
    }

    if cyclic_sccs.is_empty() {
        println!("  {}", "No cyclic clusters found (DAG).".green());
    } else {
        println!(
            "  {} cyclic cluster{}:",
            format!("{}", cyclic_sccs.len()).red().bold(),
            if cyclic_sccs.len() == 1 { "" } else { "s" }
        );
        for (i, scc) in cyclic_sccs.iter().enumerate() {
            println!();
            println!(
                "    {} (size {})",
                format!("Cluster #{}", i + 1).yellow().bold(),
                scc.len()
            );
            for member in *scc {
                println!("      {}", member.dimmed());
            }
        }
    }
}

/// Render one metric section: heading, optional verbose lines, sorted bar chart.
fn render_metric_section(
    label: &str,
    sorted_items: &[(&String, f64)],
    bar_budget: usize,
    top_n: usize,
    verbose: bool,
    verbose_lines: &[&str],
) {
    println!("\n  {}", label.bold().underline());

    if verbose {
        render::verbose_block(verbose_lines);
    }

    println!();
    if sorted_items.is_empty() {
        println!("    {}", "No clustering detected.".dimmed());
        return;
    }

    let max_val = sorted_items
        .first()
        .map(|(_, v)| *v)
        .unwrap_or(1.0)
        .max(f64::MIN_POSITIVE);

    for (name, val) in sorted_items.iter().take(top_n) {
        if *val < f64::EPSILON {
            continue;
        }
        let bar_len = ((*val / max_val) * bar_budget as f64).ceil() as usize;
        let bar = "█".repeat(bar_len.max(1));
        let ratio = (*val / max_val).min(1.0);
        let colored_bar = render::bar_color(&bar, 1.0 - ratio);
        println!("    {:<40} {} {:.4}", name.dimmed(), colored_bar, val,);
    }
}

fn render_node_table(analysis: &GraphAnalysis, top_n: usize, term_w: usize, verbose: bool) {
    if analysis.node_metrics.is_empty() {
        return;
    }

    let bar_budget = 20.min(term_w.saturating_sub(70));

    // Betweenness centrality
    let mut bc_sorted: Vec<(&String, f64)> = analysis
        .node_metrics
        .iter()
        .map(|(n, m)| (n, m.betweenness))
        .collect();
    bc_sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    render_metric_section(
        "Betweenness Centrality (bridges/bottlenecks)",
        &bc_sorted,
        bar_budget,
        top_n,
        verbose,
        &[
            "Betweenness centrality (Brandes algorithm): for each node, the fraction of",
            "all shortest paths in the graph that pass through it.",
            "Scale: 0.0 = no shortest paths traverse this node; 1.0 = all paths do.",
            "High bc = bridge/bottleneck — removing this node would disconnect or lengthen",
            "paths between other parts of the graph. Normalized by (n-1)(n-2).",
            "Bar length proportional to highest bc value. Color: red = top, green = low.",
        ],
    );

    // PageRank
    let mut pr_sorted: Vec<(&String, f64)> = analysis
        .node_metrics
        .iter()
        .map(|(n, m)| (n, m.pagerank))
        .collect();
    pr_sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    render_metric_section(
        "PageRank (recursive importance)",
        &pr_sorted,
        bar_budget,
        top_n,
        verbose,
        &[
            "PageRank: iterative importance score. A node is important if important nodes",
            "point to it. Damping factor = 0.85, 100 iterations.",
            "Dangling nodes (no outgoing edges) distribute their rank uniformly.",
            "Higher value = more central/important in the call graph.",
            "All PageRank values sum to 1.0 across the entire graph.",
        ],
    );

    // Clustering coefficient
    let mut cc_sorted: Vec<(&String, f64)> = analysis
        .node_metrics
        .iter()
        .map(|(n, m)| (n, m.clustering))
        .filter(|(_, v)| *v > f64::EPSILON)
        .collect();
    cc_sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    render_metric_section(
        "Clustering Coefficient (neighborhood density)",
        &cc_sorted,
        bar_budget,
        top_n,
        verbose,
        &[
            "Clustering coefficient: for each node, the fraction of its neighbors that",
            "are also connected to each other. Neighbors = union of successors and predecessors.",
            "Scale: 0.0 = none of the node's neighbors know each other; 1.0 = complete clique.",
            "High cc = tight neighborhood cluster. Low cc = broker between different groups.",
            "Only nodes with cc > 0 are shown.",
        ],
    );

    println!();
}

// ── Public entry point ─────────────────────────────────────────────

/// Compact summary data for embedding in the dashboard.
pub struct GraphSummary {
    /// Node with highest betweenness centrality (bridge).
    pub top_bridge: Option<(String, f64)>,
    /// Node with highest PageRank (hub).
    pub top_hub: Option<(String, f64)>,
    /// Node with highest clustering coefficient.
    pub top_clustered: Option<(String, f64)>,
}

/// Compute compact graph summary from the call graph for dashboard use.
pub fn graph_summary(rs_files: &[PathBuf], project_path: &Path) -> Option<GraphSummary> {
    let cg = flow::build_call_graph(rs_files, project_path);
    if cg.functions.is_empty() {
        return None;
    }
    let dg = digraph_from_call_graph(&cg);
    let analysis = analyze_graph(&dg, "");

    let top_bridge = analysis
        .node_metrics
        .iter()
        .filter(|(_, m)| m.betweenness > f64::EPSILON)
        .max_by(|a, b| {
            a.1.betweenness
                .partial_cmp(&b.1.betweenness)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(n, m)| (n.clone(), m.betweenness));

    let top_hub = analysis
        .node_metrics
        .iter()
        .max_by(|a, b| {
            a.1.pagerank
                .partial_cmp(&b.1.pagerank)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(n, m)| (n.clone(), m.pagerank));

    let top_clustered = analysis
        .node_metrics
        .iter()
        .filter(|(_, m)| m.clustering > f64::EPSILON)
        .max_by(|a, b| {
            a.1.clustering
                .partial_cmp(&b.1.clustering)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(n, m)| (n.clone(), m.clustering));

    Some(GraphSummary {
        top_bridge,
        top_hub,
        top_clustered,
    })
}

pub fn render_graph(
    rs_files: &[PathBuf],
    project_path: &Path,
    call_only: bool,
    dep_only: bool,
    top_n: usize,
    verbose: bool,
) {
    if !dep_only {
        let cg = flow::build_call_graph(rs_files, project_path);
        if cg.functions.is_empty() {
            println!("{}", "No functions found for call graph analysis.".yellow());
        } else {
            let dg = digraph_from_call_graph(&cg);
            let analysis = analyze_graph(&dg, "Call Graph Analysis");
            render_analysis(&analysis, top_n, verbose);
        }
    }

    if !call_only {
        let da = deps::analyze_deps(rs_files, project_path);
        if da.modules.is_empty() {
            println!(
                "{}",
                "No modules found for dependency graph analysis.".yellow()
            );
        } else {
            let dg = digraph_from_dep_analysis(&da);
            let analysis = analyze_graph(&dg, "Module Dependency Graph Analysis");
            render_analysis(&analysis, top_n, verbose);
        }
    }
}

// ── JSON Output ────────────────────────────────────────────────────

#[derive(Serialize)]
struct NodeMetricsJson {
    node: String,
    betweenness: f64,
    pagerank: f64,
    clustering: f64,
}

#[derive(Serialize)]
struct GraphStatsJson {
    density: f64,
    diameter: Option<usize>,
    avg_path_length: f64,
    component_count: usize,
}

#[derive(Serialize)]
struct GraphAnalysisJson {
    label: String,
    node_count: usize,
    edge_count: usize,
    sccs: Vec<Vec<String>>,
    node_metrics: Vec<NodeMetricsJson>,
    stats: GraphStatsJson,
}

#[derive(Serialize)]
struct GraphOutputJson {
    cstat_version: String,
    call_graph: Option<GraphAnalysisJson>,
    dep_graph: Option<GraphAnalysisJson>,
}

fn analysis_to_json(analysis: &GraphAnalysis) -> GraphAnalysisJson {
    let mut node_metrics: Vec<NodeMetricsJson> = analysis
        .node_metrics
        .iter()
        .map(|(n, m)| NodeMetricsJson {
            node: n.clone(),
            betweenness: m.betweenness,
            pagerank: m.pagerank,
            clustering: m.clustering,
        })
        .collect();
    node_metrics.sort_by(|a, b| {
        b.betweenness
            .partial_cmp(&a.betweenness)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    GraphAnalysisJson {
        label: analysis.label.clone(),
        node_count: analysis.node_count,
        edge_count: analysis.edge_count,
        sccs: analysis.sccs.clone(),
        node_metrics,
        stats: GraphStatsJson {
            density: analysis.stats.density,
            diameter: analysis.stats.diameter,
            avg_path_length: analysis.stats.avg_path_length,
            component_count: analysis.stats.component_count,
        },
    }
}

/// Render graph analysis as JSON.
pub fn render_graph_json(
    rs_files: &[PathBuf],
    project_path: &Path,
    call_only: bool,
    dep_only: bool,
) {
    let call_graph_json = if !dep_only {
        let cg = flow::build_call_graph(rs_files, project_path);
        if cg.functions.is_empty() {
            None
        } else {
            let dg = digraph_from_call_graph(&cg);
            let analysis = analyze_graph(&dg, "Call Graph Analysis");
            Some(analysis_to_json(&analysis))
        }
    } else {
        None
    };

    let dep_graph_json = if !call_only {
        let da = deps::analyze_deps(rs_files, project_path);
        if da.modules.is_empty() {
            None
        } else {
            let dg = digraph_from_dep_analysis(&da);
            let analysis = analyze_graph(&dg, "Module Dependency Graph Analysis");
            Some(analysis_to_json(&analysis))
        }
    } else {
        None
    };

    let output = GraphOutputJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        call_graph: call_graph_json,
        dep_graph: dep_graph_json,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}
