use colored::Colorize;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::deps;
use crate::flow;
use crate::render;

// ── Generic directed graph ─────────────────────────────────────────

/// A simple directed graph used for metric computation.
struct DiGraph {
    nodes: Vec<String>,
    adj: HashMap<String, Vec<String>>,     // forward edges
    rev_adj: HashMap<String, Vec<String>>, // reverse edges
}

impl DiGraph {
    fn from_edges(nodes: Vec<String>, edges: &[(String, String)]) -> Self {
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();
        let mut rev_adj: HashMap<String, Vec<String>> = HashMap::new();
        for n in &nodes {
            adj.entry(n.clone()).or_default();
            rev_adj.entry(n.clone()).or_default();
        }
        for (from, to) in edges {
            adj.entry(from.clone()).or_default().push(to.clone());
            rev_adj.entry(to.clone()).or_default().push(from.clone());
        }
        DiGraph { nodes, adj, rev_adj }
    }

    fn node_count(&self) -> usize {
        self.nodes.len()
    }

    fn edge_count(&self) -> usize {
        self.adj.values().map(|v| v.len()).sum()
    }
}

// ── Tarjan's SCC ───────────────────────────────────────────────────

struct TarjanState {
    index_counter: usize,
    stack: Vec<String>,
    on_stack: HashSet<String>,
    index: HashMap<String, usize>,
    lowlink: HashMap<String, usize>,
    sccs: Vec<Vec<String>>,
}

fn tarjan_scc(graph: &DiGraph) -> Vec<Vec<String>> {
    let mut state = TarjanState {
        index_counter: 0,
        stack: Vec::new(),
        on_stack: HashSet::new(),
        index: HashMap::new(),
        lowlink: HashMap::new(),
        sccs: Vec::new(),
    };
    for node in &graph.nodes {
        if !state.index.contains_key(node) {
            tarjan_strongconnect(node, graph, &mut state);
        }
    }
    state.sccs
}

fn tarjan_strongconnect(v: &str, graph: &DiGraph, state: &mut TarjanState) {
    state.index.insert(v.to_string(), state.index_counter);
    state.lowlink.insert(v.to_string(), state.index_counter);
    state.index_counter += 1;
    state.stack.push(v.to_string());
    state.on_stack.insert(v.to_string());

    if let Some(neighbors) = graph.adj.get(v) {
        for w in neighbors {
            if !state.index.contains_key(w.as_str()) {
                tarjan_strongconnect(w, graph, state);
                let wll = state.lowlink[w.as_str()];
                let vll = state.lowlink.get_mut(v).unwrap();
                if wll < *vll {
                    *vll = wll;
                }
            } else if state.on_stack.contains(w.as_str()) {
                let wi = state.index[w.as_str()];
                let vll = state.lowlink.get_mut(v).unwrap();
                if wi < *vll {
                    *vll = wi;
                }
            }
        }
    }

    if state.lowlink[v] == state.index[v] {
        let mut component = Vec::new();
        loop {
            let w = state.stack.pop().unwrap();
            state.on_stack.remove(&w);
            component.push(w.clone());
            if w == v {
                break;
            }
        }
        state.sccs.push(component);
    }
}

// ── Brandes' betweenness centrality ────────────────────────────────

fn betweenness_centrality(graph: &DiGraph) -> HashMap<String, f64> {
    let mut cb: HashMap<String, f64> = HashMap::new();
    for n in &graph.nodes {
        cb.insert(n.clone(), 0.0);
    }

    for s in &graph.nodes {
        let mut stack: Vec<String> = Vec::new();
        let mut predecessors: HashMap<String, Vec<String>> = HashMap::new();
        let mut sigma: HashMap<String, f64> = HashMap::new();
        let mut dist: HashMap<String, i64> = HashMap::new();

        for n in &graph.nodes {
            predecessors.insert(n.clone(), Vec::new());
            sigma.insert(n.clone(), 0.0);
            dist.insert(n.clone(), -1);
        }
        *sigma.get_mut(s).unwrap() = 1.0;
        *dist.get_mut(s).unwrap() = 0;

        let mut queue: VecDeque<String> = VecDeque::new();
        queue.push_back(s.clone());

        while let Some(v) = queue.pop_front() {
            stack.push(v.clone());
            let d_v = dist[&v];
            if let Some(neighbors) = graph.adj.get(&v) {
                for w in neighbors {
                    if dist[w] < 0 {
                        *dist.get_mut(w).unwrap() = d_v + 1;
                        queue.push_back(w.clone());
                    }
                    if dist[w] == d_v + 1 {
                        *sigma.get_mut(w).unwrap() += sigma[&v];
                        predecessors.get_mut(w).unwrap().push(v.clone());
                    }
                }
            }
        }

        let mut delta: HashMap<String, f64> = HashMap::new();
        for n in &graph.nodes {
            delta.insert(n.clone(), 0.0);
        }

        while let Some(w) = stack.pop() {
            for v in &predecessors[&w] {
                let d = sigma[v] / sigma[&w] * (1.0 + delta[&w]);
                *delta.get_mut(v).unwrap() += d;
            }
            if w != *s {
                *cb.get_mut(&w).unwrap() += delta[&w];
            }
        }
    }

    // Normalize
    let n = graph.node_count() as f64;
    if n > 2.0 {
        let norm = (n - 1.0) * (n - 2.0);
        for v in cb.values_mut() {
            *v /= norm;
        }
    }

    cb
}

// ── PageRank ───────────────────────────────────────────────────────

fn pagerank(graph: &DiGraph, damping: f64, iterations: usize) -> HashMap<String, f64> {
    let n = graph.node_count();
    if n == 0 {
        return HashMap::new();
    }
    let initial = 1.0 / n as f64;
    let mut rank: HashMap<String, f64> = graph
        .nodes
        .iter()
        .map(|n| (n.clone(), initial))
        .collect();

    for _ in 0..iterations {
        let mut new_rank: HashMap<String, f64> = HashMap::new();
        let mut dangling_sum = 0.0;

        for node in &graph.nodes {
            let out_deg = graph.adj.get(node).map_or(0, |v| v.len());
            if out_deg == 0 {
                dangling_sum += rank[node];
            }
        }

        for node in &graph.nodes {
            let mut incoming_sum = 0.0;
            if let Some(preds) = graph.rev_adj.get(node) {
                for pred in preds {
                    let out_deg = graph.adj.get(pred).map_or(1, |v| v.len().max(1));
                    incoming_sum += rank[pred] / out_deg as f64;
                }
            }
            let pr = (1.0 - damping) / n as f64
                + damping * (incoming_sum + dangling_sum / n as f64);
            new_rank.insert(node.clone(), pr);
        }
        rank = new_rank;
    }
    rank
}

// ── Clustering coefficient ─────────────────────────────────────────

fn clustering_coefficient(graph: &DiGraph) -> HashMap<String, f64> {
    let mut cc: HashMap<String, f64> = HashMap::new();

    for node in &graph.nodes {
        // Neighbors = union of successors and predecessors
        let mut neighbors: HashSet<&str> = HashSet::new();
        if let Some(succs) = graph.adj.get(node) {
            for s in succs {
                neighbors.insert(s.as_str());
            }
        }
        if let Some(preds) = graph.rev_adj.get(node) {
            for p in preds {
                neighbors.insert(p.as_str());
            }
        }
        neighbors.remove(node.as_str());

        let k = neighbors.len();
        if k < 2 {
            cc.insert(node.clone(), 0.0);
            continue;
        }

        let mut links = 0usize;
        let neighbor_vec: Vec<&str> = neighbors.iter().copied().collect();
        for i in 0..neighbor_vec.len() {
            for j in 0..neighbor_vec.len() {
                if i == j {
                    continue;
                }
                if let Some(adj) = graph.adj.get(neighbor_vec[i]) {
                    if adj.iter().any(|x| x == neighbor_vec[j]) {
                        links += 1;
                    }
                }
            }
        }

        let possible = k * (k - 1); // directed pairs
        cc.insert(node.clone(), links as f64 / possible as f64);
    }
    cc
}

// ── Graph-level stats ──────────────────────────────────────────────

struct GraphStats {
    density: f64,
    diameter: Option<usize>,
    avg_path_length: f64,
    component_count: usize,
}

fn graph_level_stats(graph: &DiGraph) -> GraphStats {
    let n = graph.node_count();
    let m = graph.edge_count();

    let density = if n > 1 {
        m as f64 / (n as f64 * (n as f64 - 1.0))
    } else {
        0.0
    };

    // BFS from every node to compute diameter and avg path length
    // Only do this for reasonably sized graphs
    let (diameter, avg_path_length) = if n <= 2000 {
        compute_distances(graph)
    } else {
        (None, 0.0)
    };

    // Weakly connected components via union-find style BFS on undirected view
    let component_count = count_weak_components(graph);

    GraphStats {
        density,
        diameter,
        avg_path_length,
        component_count,
    }
}

fn compute_distances(graph: &DiGraph) -> (Option<usize>, f64) {
    let mut max_dist = 0usize;
    let mut total_dist = 0u64;
    let mut pair_count = 0u64;

    for s in &graph.nodes {
        let mut dist: HashMap<&str, usize> = HashMap::new();
        dist.insert(s.as_str(), 0);
        let mut queue: VecDeque<&str> = VecDeque::new();
        queue.push_back(s.as_str());

        while let Some(v) = queue.pop_front() {
            let d = dist[v];
            if let Some(neighbors) = graph.adj.get(v) {
                for w in neighbors {
                    if !dist.contains_key(w.as_str()) {
                        let nd = d + 1;
                        dist.insert(w.as_str(), nd);
                        queue.push_back(w.as_str());
                        if nd > max_dist {
                            max_dist = nd;
                        }
                        total_dist += nd as u64;
                        pair_count += 1;
                    }
                }
            }
        }
    }

    let diameter = if pair_count > 0 { Some(max_dist) } else { None };
    let avg = if pair_count > 0 {
        total_dist as f64 / pair_count as f64
    } else {
        0.0
    };
    (diameter, avg)
}

fn count_weak_components(graph: &DiGraph) -> usize {
    let mut visited: HashSet<&str> = HashSet::new();
    let mut count = 0;

    for node in &graph.nodes {
        if visited.contains(node.as_str()) {
            continue;
        }
        count += 1;
        // BFS on undirected view
        let mut queue: VecDeque<&str> = VecDeque::new();
        queue.push_back(node.as_str());
        visited.insert(node.as_str());

        while let Some(v) = queue.pop_front() {
            if let Some(neighbors) = graph.adj.get(v) {
                for w in neighbors {
                    if !visited.contains(w.as_str()) {
                        visited.insert(w.as_str());
                        queue.push_back(w.as_str());
                    }
                }
            }
            if let Some(neighbors) = graph.rev_adj.get(v) {
                for w in neighbors {
                    if !visited.contains(w.as_str()) {
                        visited.insert(w.as_str());
                        queue.push_back(w.as_str());
                    }
                }
            }
        }
    }
    count
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
    let edges: Vec<(String, String)> = da.edges.iter().map(|e| (e.from.clone(), e.to.clone())).collect();
    DiGraph::from_edges(nodes, &edges)
}

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

// ── Rendering ──────────────────────────────────────────────────────

fn render_analysis(analysis: &GraphAnalysis, top_n: usize, verbose: bool) {
    let term_w = render::terminal_width();

    // Header
    let header = format!("── {} ─", analysis.label);
    let padded = format!("{:─<width$}", header, width = 56);
    println!("\n{}", padded.bright_cyan().bold());

    // Graph-level stats box
    render_stats_box(analysis, verbose);

    // SCCs
    render_sccs(analysis, verbose);

    // Per-node metrics
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

    println!(
        "\n  {}",
        "Strongly Connected Components".bold().underline()
    );

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

fn render_node_table(analysis: &GraphAnalysis, top_n: usize, term_w: usize, verbose: bool) {
    if analysis.node_metrics.is_empty() {
        return;
    }

    let bar_budget = 20.min(term_w.saturating_sub(70));

    // Betweenness centrality — top bridges
    println!(
        "\n  {}",
        "Betweenness Centrality (bridges/bottlenecks)".bold().underline()
    );

    if verbose {
        render::verbose_block(&[
            "Betweenness centrality (Brandes algorithm): for each node, the fraction of",
            "all shortest paths in the graph that pass through it.",
            "Scale: 0.0 = no shortest paths traverse this node; 1.0 = all paths do.",
            "High bc = bridge/bottleneck — removing this node would disconnect or lengthen",
            "paths between other parts of the graph. Normalized by (n-1)(n-2).",
            "Bar length proportional to highest bc value. Color: red = top, green = low.",
        ]);
    }
    let mut bc_sorted: Vec<(&String, f64)> = analysis
        .node_metrics
        .iter()
        .map(|(n, m)| (n, m.betweenness))
        .collect();
    bc_sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let bc_max = bc_sorted.first().map(|(_, v)| *v).unwrap_or(1.0).max(f64::MIN_POSITIVE);

    println!();
    for (name, val) in bc_sorted.iter().take(top_n) {
        if *val < f64::EPSILON {
            continue;
        }
        let bar_len = ((*val / bc_max) * bar_budget as f64).ceil() as usize;
        let bar = "█".repeat(bar_len.max(1));
        let ratio = (*val / bc_max).min(1.0);
        let colored_bar = render::bar_color(&bar, 1.0 - ratio);
        println!(
            "    {:<40} {} {:.4}",
            name.dimmed(),
            colored_bar,
            val,
        );
    }

    // PageRank — recursive importance
    println!(
        "\n  {}",
        "PageRank (recursive importance)".bold().underline()
    );

    if verbose {
        render::verbose_block(&[
            "PageRank: iterative importance score. A node is important if important nodes",
            "point to it. Damping factor = 0.85, 100 iterations.",
            "Dangling nodes (no outgoing edges) distribute their rank uniformly.",
            "Higher value = more central/important in the call graph.",
            "All PageRank values sum to 1.0 across the entire graph.",
        ]);
    }
    let mut pr_sorted: Vec<(&String, f64)> = analysis
        .node_metrics
        .iter()
        .map(|(n, m)| (n, m.pagerank))
        .collect();
    pr_sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let pr_max = pr_sorted.first().map(|(_, v)| *v).unwrap_or(1.0).max(f64::MIN_POSITIVE);

    println!();
    for (name, val) in pr_sorted.iter().take(top_n) {
        let bar_len = ((*val / pr_max) * bar_budget as f64).ceil() as usize;
        let bar = "█".repeat(bar_len.max(1));
        let ratio = (*val / pr_max).min(1.0);
        let colored_bar = render::bar_color(&bar, 1.0 - ratio);
        println!(
            "    {:<40} {} {:.4}",
            name.dimmed(),
            colored_bar,
            val,
        );
    }

    // Clustering coefficient — cliqueyness
    println!(
        "\n  {}",
        "Clustering Coefficient (neighborhood density)".bold().underline()
    );

    if verbose {
        render::verbose_block(&[
            "Clustering coefficient: for each node, the fraction of its neighbors that",
            "are also connected to each other. Neighbors = union of successors and predecessors.",
            "Scale: 0.0 = none of the node's neighbors know each other; 1.0 = complete clique.",
            "High cc = tight neighborhood cluster. Low cc = broker between different groups.",
            "Only nodes with cc > 0 are shown.",
        ]);
    }
    let mut cc_sorted: Vec<(&String, f64)> = analysis
        .node_metrics
        .iter()
        .map(|(n, m)| (n, m.clustering))
        .filter(|(_, v)| *v > f64::EPSILON)
        .collect();
    cc_sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    println!();
    if cc_sorted.is_empty() {
        println!("    {}", "No clustering detected.".dimmed());
    } else {
        let cc_max = cc_sorted.first().map(|(_, v)| *v).unwrap_or(1.0).max(f64::MIN_POSITIVE);
        for (name, val) in cc_sorted.iter().take(top_n) {
            let bar_len = ((*val / cc_max) * bar_budget as f64).ceil() as usize;
            let bar = "█".repeat(bar_len.max(1));
            let ratio = (*val / cc_max).min(1.0);
            let colored_bar = render::bar_color(&bar, 1.0 - ratio);
            println!(
                "    {:<40} {} {:.4}",
                name.dimmed(),
                colored_bar,
                val,
            );
        }
    }

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
        .max_by(|a, b| a.1.betweenness.partial_cmp(&b.1.betweenness).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(n, m)| (n.clone(), m.betweenness));

    let top_hub = analysis
        .node_metrics
        .iter()
        .max_by(|a, b| a.1.pagerank.partial_cmp(&b.1.pagerank).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(n, m)| (n.clone(), m.pagerank));

    let top_clustered = analysis
        .node_metrics
        .iter()
        .filter(|(_, m)| m.clustering > f64::EPSILON)
        .max_by(|a, b| a.1.clustering.partial_cmp(&b.1.clustering).unwrap_or(std::cmp::Ordering::Equal))
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
            println!("{}", "No modules found for dependency graph analysis.".yellow());
        } else {
            let dg = digraph_from_dep_analysis(&da);
            let analysis = analyze_graph(&dg, "Module Dependency Graph Analysis");
            render_analysis(&analysis, top_n, verbose);
        }
    }
}

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
    node_metrics.sort_by(|a, b| b.betweenness.partial_cmp(&a.betweenness).unwrap_or(std::cmp::Ordering::Equal));

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
