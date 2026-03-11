use std::collections::{HashMap, HashSet, VecDeque};

// ── Generic directed graph ─────────────────────────────────────────

/// A simple directed graph used for metric computation.
pub(super) struct DiGraph {
    pub(super) nodes: Vec<String>,
    pub(super) adj: HashMap<String, Vec<String>>,     // forward edges
    pub(super) rev_adj: HashMap<String, Vec<String>>, // reverse edges
}

impl DiGraph {
    pub(super) fn from_edges(nodes: Vec<String>, edges: &[(String, String)]) -> Self {
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

    pub(super) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub(super) fn edge_count(&self) -> usize {
        self.adj.values().map(|v| v.len()).sum()
    }

    pub(super) fn density(&self) -> f64 {
        let n = self.node_count();
        if n <= 1 {
            return 0.0;
        }
        self.edge_count() as f64 / (n * (n - 1)) as f64
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

pub(super) fn tarjan_scc(graph: &DiGraph) -> Vec<Vec<String>> {
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

/// Result of BFS forward pass from a single source.
struct BfsResult {
    stack: Vec<String>,
    predecessors: HashMap<String, Vec<String>>,
    sigma: HashMap<String, f64>,
}

/// BFS forward pass: compute shortest-path counts and predecessor lists from source `s`.
fn bfs_shortest_paths(s: &str, graph: &DiGraph) -> BfsResult {
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
    queue.push_back(s.to_string());

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

    BfsResult { stack, predecessors, sigma }
}

/// Back-propagation: accumulate betweenness from dependency scores.
fn accumulate_betweenness(
    mut stack: Vec<String>,
    predecessors: &HashMap<String, Vec<String>>,
    sigma: &HashMap<String, f64>,
    s: &str,
    cb: &mut HashMap<String, f64>,
) {
    let mut delta: HashMap<String, f64> = HashMap::new();
    for n in sigma.keys() {
        delta.insert(n.clone(), 0.0);
    }

    while let Some(w) = stack.pop() {
        for v in &predecessors[&w] {
            let d = sigma[v] / sigma[&w] * (1.0 + delta[&w]);
            *delta.get_mut(v).unwrap() += d;
        }
        if w != s {
            *cb.get_mut(&w).unwrap() += delta[&w];
        }
    }
}

pub(super) fn betweenness_centrality(graph: &DiGraph) -> HashMap<String, f64> {
    let mut cb: HashMap<String, f64> = HashMap::new();
    for n in &graph.nodes {
        cb.insert(n.clone(), 0.0);
    }

    for s in &graph.nodes {
        let bfs = bfs_shortest_paths(s, graph);
        accumulate_betweenness(bfs.stack, &bfs.predecessors, &bfs.sigma, s, &mut cb);
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

pub(super) fn pagerank(graph: &DiGraph, damping: f64, iterations: usize) -> HashMap<String, f64> {
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

/// Count directed links that exist between members of `neighbors` in the adjacency map.
fn count_neighbor_links(neighbors: &[&str], adj: &HashMap<String, Vec<String>>) -> usize {
    let mut links = 0usize;
    for i in 0..neighbors.len() {
        for j in 0..neighbors.len() {
            if i == j {
                continue;
            }
            if let Some(succs) = adj.get(neighbors[i]) {
                if succs.iter().any(|x| x == neighbors[j]) {
                    links += 1;
                }
            }
        }
    }
    links
}

pub(super) fn clustering_coefficient(graph: &DiGraph) -> HashMap<String, f64> {
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

        let neighbor_vec: Vec<&str> = neighbors.iter().copied().collect();
        let links = count_neighbor_links(&neighbor_vec, &graph.adj);

        let possible = k * (k - 1); // directed pairs
        cc.insert(node.clone(), links as f64 / possible as f64);
    }
    cc
}

// ── Graph-level stats ──────────────────────────────────────────────

pub(super) struct GraphStats {
    pub(super) density: f64,
    pub(super) diameter: Option<usize>,
    pub(super) avg_path_length: f64,
    pub(super) component_count: usize,
}

pub(super) fn graph_level_stats(graph: &DiGraph) -> GraphStats {
    let density = graph.density();

    let (diameter, avg_path_length) = if graph.node_count() <= 2000 {
        compute_distances(graph)
    } else {
        (None, 0.0)
    };

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

/// BFS traversal for one weak component starting at `start`.
/// Visits all nodes reachable via forward or reverse edges, marking them in `visited`.
fn bfs_weak_component<'a>(
    start: &'a str,
    graph: &'a DiGraph,
    visited: &mut HashSet<&'a str>,
) {
    let mut queue: VecDeque<&str> = VecDeque::new();
    queue.push_back(start);
    visited.insert(start);

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

fn count_weak_components(graph: &DiGraph) -> usize {
    let mut visited: HashSet<&str> = HashSet::new();
    let mut count = 0;

    for node in &graph.nodes {
        if visited.contains(node.as_str()) {
            continue;
        }
        count += 1;
        bfs_weak_component(node.as_str(), graph, &mut visited);
    }
    count
}
