//! Spec §2.7 graph-derived representations reachable from stable-side data.
//!
//! Stage 9 covers levels 31 (module-dep graph), 32 (intra-crate static
//! call graph), 34 (per-function CFG approximation), 36 (type-usage
//! bipartite graph), 37 (SCCs), 38 (community decomposition + modularity
//! Q), 39 (power-law fit of degree distributions), and 40 (self-similarity
//! / cross-scale shape comparison). Levels 33 (mono call graph) and 35
//! (data-flow) wait for codegen-side data and arrive in Stage 14.
//!
//! All eight levels read from a shared [`SharedGraphs`] cache so the
//! second-parse of every `.rs` file happens at most once per run.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use cstat_orchestrate::{Context, CostClass, EntityRecord, Level, LevelError, LevelOutput};
use cstat_schema::{AstFuncId, FileId, IdSet, ModuleId};

use crate::ast_cache::SharedAst;
use crate::graph_cache::SharedGraphs;

// ───── Level 31: module dependency graph ────────────────────────────────

pub(crate) struct GraphModLevel {
    pub shared: SharedGraphs,
}

impl Level for GraphModLevel {
    fn id(&self) -> &'static str { "graph.mod" }
    fn cost_class(&self) -> CostClass { CostClass::L }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let g = self.shared.get(&ctx.project_root);
        let n = g.modules.len();
        let m = g.module_edges.len();
        let degrees = directed_degrees(n, &g.module_edges);

        let (in_max, in_mean, _) = summarize_u64(&degrees.in_deg);
        let (out_max, out_mean, _) = summarize_u64(&degrees.out_deg);

        let density = if n > 1 {
            m as f64 / (n as f64 * (n as f64 - 1.0))
        } else {
            0.0
        };

        let vector = vec![
            ("graph.mod.node_count".into(), n as f64),
            ("graph.mod.edge_count".into(), m as f64),
            ("graph.mod.density".into(), density),
            ("graph.mod.in_degree.max".into(), in_max),
            ("graph.mod.in_degree.mean".into(), in_mean),
            ("graph.mod.out_degree.max".into(), out_max),
            ("graph.mod.out_degree.mean".into(), out_mean),
        ];

        let mut module_records: Vec<EntityRecord> = Vec::with_capacity(n);
        for (i, module_path) in g.modules.iter().enumerate() {
            let mut metrics = BTreeMap::new();
            metrics.insert("in_degree".into(), degrees.in_deg[i] as f64);
            metrics.insert("out_degree".into(), degrees.out_deg[i] as f64);
            module_records.push(EntityRecord {
                ids: IdSet {
                    module: Some(ModuleId::new(module_path.clone())),
                    ..Default::default()
                },
                metrics,
            });
        }
        let mut entities = BTreeMap::new();
        if !module_records.is_empty() {
            entities.insert("graph.mod_node".into(), module_records);
        }

        Ok(LevelOutput { vector, entities })
    }
}

// ───── Level 32: intra-crate static call graph ──────────────────────────

pub(crate) struct GraphCallStaticLevel {
    pub shared: SharedGraphs,
}

impl Level for GraphCallStaticLevel {
    fn id(&self) -> &'static str { "graph.call_static" }
    fn cost_class(&self) -> CostClass { CostClass::L }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let g = self.shared.get(&ctx.project_root);
        let n = g.functions.len();
        let m = g.call_edges.len();
        let degrees = directed_degrees(n, &g.call_edges);
        let (in_max, in_mean, _) = summarize_u64(&degrees.in_deg);
        let (out_max, out_mean, _) = summarize_u64(&degrees.out_deg);

        let leaves = degrees.out_deg.iter().filter(|d| **d == 0).count();
        let roots = degrees.in_deg.iter().filter(|d| **d == 0).count();
        let density = if n > 1 {
            m as f64 / (n as f64 * (n as f64 - 1.0))
        } else {
            0.0
        };

        let vector = vec![
            ("graph.call_static.node_count".into(), n as f64),
            ("graph.call_static.edge_count".into(), m as f64),
            ("graph.call_static.density".into(), density),
            ("graph.call_static.leaf_count".into(), leaves as f64),
            ("graph.call_static.root_count".into(), roots as f64),
            ("graph.call_static.in_degree.max".into(), in_max),
            ("graph.call_static.in_degree.mean".into(), in_mean),
            ("graph.call_static.out_degree.max".into(), out_max),
            ("graph.call_static.out_degree.mean".into(), out_mean),
        ];
        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Level 34: CFG per function ───────────────────────────────────────

pub(crate) struct GraphCfgLevel {
    pub shared: SharedGraphs,
    pub ast: SharedAst,
}

impl Level for GraphCfgLevel {
    fn id(&self) -> &'static str { "graph.cfg" }
    fn cost_class(&self) -> CostClass { CostClass::L }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let g = self.shared.get(&ctx.project_root);
        let ast = self.ast.get(&ctx.project_root);

        let node_counts: Vec<u64> = g.cfg.iter().map(|c| c.node_count).collect();
        let edge_counts: Vec<u64> = g.cfg.iter().map(|c| c.edge_count).collect();
        let branch_counts: Vec<u64> = g.cfg.iter().map(|c| c.branch_count).collect();
        let returns: Vec<u64> = g.cfg.iter().map(|c| c.return_count).collect();

        let (nodes_max, nodes_mean, nodes_sum) = summarize_u64(&node_counts);
        let (edges_max, edges_mean, edges_sum) = summarize_u64(&edge_counts);
        let (branches_max, branches_mean, branches_sum) = summarize_u64(&branch_counts);
        let (returns_max, returns_mean, _) = summarize_u64(&returns);

        let vector = vec![
            ("graph.cfg.function_count".into(), g.cfg.len() as f64),
            ("graph.cfg.node_count.sum".into(), nodes_sum),
            ("graph.cfg.node_count.max".into(), nodes_max),
            ("graph.cfg.node_count.mean".into(), nodes_mean),
            ("graph.cfg.edge_count.sum".into(), edges_sum),
            ("graph.cfg.edge_count.max".into(), edges_max),
            ("graph.cfg.edge_count.mean".into(), edges_mean),
            ("graph.cfg.branch_count.sum".into(), branches_sum),
            ("graph.cfg.branch_count.max".into(), branches_max),
            ("graph.cfg.branch_count.mean".into(), branches_mean),
            ("graph.cfg.return_count.max".into(), returns_max),
            ("graph.cfg.return_count.mean".into(), returns_mean),
        ];

        // Per-function records — co-iterate functions and ast functions
        // (same order) so we can attach an AstFuncId.
        let mut records: Vec<EntityRecord> = Vec::with_capacity(g.cfg.len());
        for (cfg_stats, fs) in g.cfg.iter().zip(ast.iter_functions()) {
            let mut metrics = BTreeMap::new();
            metrics.insert("nodes".into(), cfg_stats.node_count as f64);
            metrics.insert("edges".into(), cfg_stats.edge_count as f64);
            metrics.insert("branches".into(), cfg_stats.branch_count as f64);
            metrics.insert("returns".into(), cfg_stats.return_count as f64);
            let file = FileId::new(posix(&fs.file_path));
            records.push(EntityRecord {
                ids: IdSet {
                    file: Some(file.clone()),
                    ast_func: Some(AstFuncId {
                        file,
                        fq_path: fs.fq_path.clone(),
                        item_kind: "fn".into(),
                        ast_node_hash: fs.node_hash.clone(),
                    }),
                    ..Default::default()
                },
                metrics,
            });
        }
        let mut entities = BTreeMap::new();
        if !records.is_empty() {
            entities.insert("graph.cfg_func".into(), records);
        }

        Ok(LevelOutput { vector, entities })
    }
}

// ───── Level 36: type-usage bipartite graph ─────────────────────────────

pub(crate) struct GraphTypeUsageLevel {
    pub shared: SharedGraphs,
}

impl Level for GraphTypeUsageLevel {
    fn id(&self) -> &'static str { "graph.type_usage" }
    fn cost_class(&self) -> CostClass { CostClass::L }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let g = self.shared.get(&ctx.project_root);

        let func_count = g.functions.len();
        let type_count = g.types.len();
        let edge_count = g.type_usage_edges.len();

        // Per-type fan-in: how many functions reference this type?
        let mut fan_in = vec![0u64; type_count.max(1)];
        let mut fan_out = vec![0u64; func_count.max(1)];
        for &(fi, ti) in &g.type_usage_edges {
            if fi < fan_out.len() { fan_out[fi] += 1; }
            if ti < fan_in.len() { fan_in[ti] += 1; }
        }
        let (fi_max, fi_mean, _) = summarize_u64(&fan_in);
        let (fo_max, fo_mean, _) = summarize_u64(&fan_out);

        let density = if func_count > 0 && type_count > 0 {
            edge_count as f64 / (func_count as f64 * type_count as f64)
        } else {
            0.0
        };

        let vector = vec![
            ("graph.type_usage.function_count".into(), func_count as f64),
            ("graph.type_usage.type_count".into(), type_count as f64),
            ("graph.type_usage.edge_count".into(), edge_count as f64),
            ("graph.type_usage.density".into(), density),
            ("graph.type_usage.type_fan_in.max".into(), fi_max),
            ("graph.type_usage.type_fan_in.mean".into(), fi_mean),
            ("graph.type_usage.func_fan_out.max".into(), fo_max),
            ("graph.type_usage.func_fan_out.mean".into(), fo_mean),
        ];
        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Level 37: SCCs / cycles ──────────────────────────────────────────

pub(crate) struct GraphSccsLevel {
    pub shared: SharedGraphs,
}

impl Level for GraphSccsLevel {
    fn id(&self) -> &'static str { "graph.sccs" }
    fn cost_class(&self) -> CostClass { CostClass::L }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let g = self.shared.get(&ctx.project_root);

        let call_sccs = tarjan_scc(g.functions.len(), &g.call_edges);
        let mod_sccs = tarjan_scc(g.modules.len(), &g.module_edges);

        let call_nontrivial = call_sccs.iter().filter(|c| c.len() > 1).count();
        let mod_nontrivial = mod_sccs.iter().filter(|c| c.len() > 1).count();

        let call_largest = call_sccs.iter().map(|c| c.len()).max().unwrap_or(0);
        let mod_largest = mod_sccs.iter().map(|c| c.len()).max().unwrap_or(0);

        let vector = vec![
            ("graph.sccs.call_static.count".into(), call_sccs.len() as f64),
            ("graph.sccs.call_static.nontrivial_count".into(), call_nontrivial as f64),
            ("graph.sccs.call_static.largest_size".into(), call_largest as f64),
            ("graph.sccs.mod.count".into(), mod_sccs.len() as f64),
            ("graph.sccs.mod.nontrivial_count".into(), mod_nontrivial as f64),
            ("graph.sccs.mod.largest_size".into(), mod_largest as f64),
        ];
        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Level 38: communities + modularity Q ─────────────────────────────

pub(crate) struct GraphCommunitiesLevel {
    pub shared: SharedGraphs,
}

impl Level for GraphCommunitiesLevel {
    fn id(&self) -> &'static str { "graph.communities" }
    fn cost_class(&self) -> CostClass { CostClass::L }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let g = self.shared.get(&ctx.project_root);

        let (call_partition, call_q) =
            label_propagation(g.functions.len(), &g.call_edges);
        let (mod_partition, mod_q) =
            label_propagation(g.modules.len(), &g.module_edges);

        let call_communities = distinct(&call_partition);
        let mod_communities = distinct(&mod_partition);

        let vector = vec![
            ("graph.communities.call_static.community_count".into(), call_communities as f64),
            ("graph.communities.call_static.modularity_q".into(), call_q),
            ("graph.communities.mod.community_count".into(), mod_communities as f64),
            ("graph.communities.mod.modularity_q".into(), mod_q),
        ];
        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Level 39: power-law fit of degree distributions ──────────────────

pub(crate) struct GraphPowerLawLevel {
    pub shared: SharedGraphs,
}

impl Level for GraphPowerLawLevel {
    fn id(&self) -> &'static str { "graph.power_law" }
    fn cost_class(&self) -> CostClass { CostClass::L }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let g = self.shared.get(&ctx.project_root);

        // For both graphs, take the total-degree distribution and fit
        // an MLE power law. With few samples the fit is meaningless;
        // we report n_above_xmin so consumers can decide whether to
        // trust the exponent.
        let call_total = total_degrees(g.functions.len(), &g.call_edges);
        let mod_total = total_degrees(g.modules.len(), &g.module_edges);

        let call_fit = power_law_mle(&call_total, 1);
        let mod_fit = power_law_mle(&mod_total, 1);

        let vector = vec![
            ("graph.power_law.call_static.degree_exponent".into(), call_fit.alpha),
            ("graph.power_law.call_static.sample_count".into(), call_fit.n as f64),
            ("graph.power_law.mod.degree_exponent".into(), mod_fit.alpha),
            ("graph.power_law.mod.sample_count".into(), mod_fit.n as f64),
        ];
        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Level 40: self-similarity (cross-scale shape comparison) ─────────

pub(crate) struct GraphSelfSimLevel {
    pub shared: SharedGraphs,
}

impl Level for GraphSelfSimLevel {
    fn id(&self) -> &'static str { "graph.self_sim" }
    fn cost_class(&self) -> CostClass { CostClass::L }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let g = self.shared.get(&ctx.project_root);

        // The chosen scalar is the per-entity branch count, normalized
        // by the entity's own total. KS distance compares the empirical
        // CDFs across scales — function ↔ file ↔ module.
        let per_fn: Vec<f64> = g.cfg.iter().map(|c| c.branch_count as f64).collect();
        let per_file: Vec<f64> = g.file_branch_totals.iter().map(|x| *x as f64).collect();
        let per_module: Vec<f64> = g.module_branch_totals.iter().map(|x| *x as f64).collect();

        let fn_vs_file = ks_distance(&per_fn, &per_file);
        let fn_vs_module = ks_distance(&per_fn, &per_module);
        let file_vs_module = ks_distance(&per_file, &per_module);

        let vector = vec![
            ("graph.self_sim.branch.func_vs_file.ks_distance".into(), fn_vs_file),
            ("graph.self_sim.branch.func_vs_module.ks_distance".into(), fn_vs_module),
            ("graph.self_sim.branch.file_vs_module.ks_distance".into(), file_vs_module),
            ("graph.self_sim.function_sample_count".into(), per_fn.len() as f64),
            ("graph.self_sim.file_sample_count".into(), per_file.len() as f64),
            ("graph.self_sim.module_sample_count".into(), per_module.len() as f64),
        ];
        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Graph helpers ────────────────────────────────────────────────────

struct DirectedDegrees {
    in_deg: Vec<u64>,
    out_deg: Vec<u64>,
}

fn directed_degrees(n: usize, edges: &[(usize, usize)]) -> DirectedDegrees {
    let mut in_deg = vec![0u64; n];
    let mut out_deg = vec![0u64; n];
    for &(s, t) in edges {
        if s < n { out_deg[s] += 1; }
        if t < n { in_deg[t] += 1; }
    }
    DirectedDegrees { in_deg, out_deg }
}

fn total_degrees(n: usize, edges: &[(usize, usize)]) -> Vec<u64> {
    let DirectedDegrees { in_deg, out_deg } = directed_degrees(n, edges);
    in_deg.into_iter().zip(out_deg).map(|(a, b)| a + b).collect()
}

fn summarize_u64(values: &[u64]) -> (f64, f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let sum: u64 = values.iter().sum();
    let max = *values.iter().max().unwrap();
    let mean = sum as f64 / values.len() as f64;
    (max as f64, mean, sum as f64)
}

/// Tarjan's strongly-connected-components algorithm. Returns SCCs as a
/// `Vec<Vec<usize>>` of sorted node indices. Singletons (including isolated
/// nodes with no edges) are returned as size-1 SCCs.
///
/// Implemented iteratively to avoid stack overflow on large call graphs.
/// The `Frame::Resume(v, i)` variant is hit after the `i-1`-th neighbor's
/// DFS subtree has finished, so we propagate that child's lowlink up
/// before continuing the loop — the standard recursive pattern, unrolled.
fn tarjan_scc(n: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(s, t) in edges {
        if s < n && t < n {
            adj[s].push(t);
        }
    }
    let mut index = 0u64;
    let mut indices: Vec<Option<u64>> = vec![None; n];
    let mut lowlinks: Vec<u64> = vec![0; n];
    let mut on_stack: Vec<bool> = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut sccs: Vec<Vec<usize>> = Vec::new();

    enum Frame {
        Enter(usize),
        Resume(usize, usize),
    }

    for start in 0..n {
        if indices[start].is_some() {
            continue;
        }
        let mut work: Vec<Frame> = vec![Frame::Enter(start)];
        while let Some(frame) = work.pop() {
            match frame {
                Frame::Enter(v) => {
                    indices[v] = Some(index);
                    lowlinks[v] = index;
                    index += 1;
                    stack.push(v);
                    on_stack[v] = true;
                    work.push(Frame::Resume(v, 0));
                }
                Frame::Resume(v, mut i) => {
                    // If we're resuming after a tree-child DFS, the just-
                    // finished child is at index i-1. Propagate its
                    // lowlink up into v.
                    if i > 0 {
                        let prev = adj[v][i - 1];
                        lowlinks[v] = lowlinks[v].min(lowlinks[prev]);
                    }
                    let mut descended = false;
                    while i < adj[v].len() {
                        let w = adj[v][i];
                        match indices[w] {
                            None => {
                                // Tree edge: descend. We'll resume v at i+1
                                // and the resume-prologue above will pull
                                // up w's lowlink for us.
                                work.push(Frame::Resume(v, i + 1));
                                work.push(Frame::Enter(w));
                                descended = true;
                                break;
                            }
                            Some(idx_w) => {
                                if on_stack[w] {
                                    // Back / cross edge to an ancestor still
                                    // on the stack. Standard Tarjan: use
                                    // `index[w]` (not `lowlink[w]`) for
                                    // these so we don't cross SCC boundaries.
                                    lowlinks[v] = lowlinks[v].min(idx_w);
                                }
                                i += 1;
                            }
                        }
                    }
                    if !descended {
                        // All neighbors processed; v is fully visited.
                        if lowlinks[v] == indices[v].unwrap() {
                            let mut scc = Vec::new();
                            loop {
                                let w = stack.pop().expect("stack non-empty inside SCC pop");
                                on_stack[w] = false;
                                scc.push(w);
                                if w == v {
                                    break;
                                }
                            }
                            scc.sort_unstable();
                            sccs.push(scc);
                        }
                    }
                }
            }
        }
    }

    sccs
}

/// Label-propagation community detection on an undirected projection of
/// the graph. Returns `(partition, modularity_q)`.
///
/// Deterministic by construction: nodes are visited in index order each
/// round; ties between candidate labels are broken in favor of the
/// numerically lowest label. Iterates up to 32 rounds or until stable.
fn label_propagation(n: usize, edges: &[(usize, usize)]) -> (Vec<usize>, f64) {
    if n == 0 {
        return (Vec::new(), 0.0);
    }
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(s, t) in edges {
        if s < n && t < n && s != t {
            adj[s].push(t);
            adj[t].push(s);
        }
    }

    let mut label: Vec<usize> = (0..n).collect();
    for _round in 0..32 {
        let mut changed = false;
        for v in 0..n {
            if adj[v].is_empty() {
                continue;
            }
            let mut counts: BTreeMap<usize, u64> = BTreeMap::new();
            for &u in &adj[v] {
                *counts.entry(label[u]).or_insert(0) += 1;
            }
            let best = counts
                .iter()
                .max_by(|a, b| {
                    a.1.cmp(b.1)
                        .then_with(|| b.0.cmp(a.0)) // smaller label wins ties
                })
                .map(|(l, _)| *l)
                .unwrap_or(label[v]);
            if best != label[v] {
                label[v] = best;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let q = modularity_q(n, edges, &label);
    (label, q)
}

fn distinct<T: Ord + Clone>(values: &[T]) -> usize {
    let s: BTreeSet<T> = values.iter().cloned().collect();
    s.len()
}

/// Newman's modularity Q on an undirected graph.
///
/// Q = (1 / 2m) Σ_{ij} [A_ij - k_i k_j / 2m] δ(c_i, c_j)
///
/// where m is the total edge weight (undirected sum), A_ij is the
/// adjacency, and k_i is the degree of node i. Returns 0.0 when the
/// graph has no edges.
fn modularity_q(n: usize, edges: &[(usize, usize)], label: &[usize]) -> f64 {
    if edges.is_empty() || n == 0 {
        return 0.0;
    }
    // Undirected adjacency (sum of both directions) and degrees.
    let mut deg = vec![0i64; n];
    let mut edge_set: BTreeMap<(usize, usize), i64> = BTreeMap::new();
    for &(s, t) in edges {
        if s >= n || t >= n {
            continue;
        }
        let (a, b) = if s <= t { (s, t) } else { (t, s) };
        *edge_set.entry((a, b)).or_insert(0) += 1;
        deg[s] += 1;
        deg[t] += 1;
    }
    let m_undirected: i64 = edge_set.values().sum();
    if m_undirected == 0 {
        return 0.0;
    }
    let two_m = (2 * m_undirected) as f64;

    let mut q = 0.0;
    // Internal edges: pairs in the same community get +A_ij contribution
    // (counted twice since the formula is over ordered (i,j) pairs).
    for ((a, b), w) in &edge_set {
        let contrib = *w as f64;
        if label[*a] == label[*b] {
            // Internal undirected edge contributes +2 * w to numerator
            // (i,j) and (j,i). Self-loops only count once.
            q += if a == b { contrib } else { 2.0 * contrib };
        }
    }
    // Subtract k_i k_j / 2m for all same-community pairs.
    let mut community_deg: BTreeMap<usize, i64> = BTreeMap::new();
    for v in 0..n {
        *community_deg.entry(label[v]).or_insert(0) += deg[v];
    }
    for k in community_deg.values() {
        let ki = *k as f64;
        q -= (ki * ki) / two_m;
    }
    q / two_m
}

#[derive(Debug, Clone, Copy)]
struct PowerLawFit {
    alpha: f64,
    n: usize,
}

/// Maximum-likelihood estimate of the power-law exponent over
/// `values >= x_min`. Returns α = 1 + n / Σ ln(x_i / (x_min - 0.5))
/// (Clauset/Shalizi/Newman discrete MLE). If fewer than 2 samples are
/// at-or-above `x_min`, returns alpha = 0.0 to signal an unfittable
/// distribution.
fn power_law_mle(values: &[u64], x_min: u64) -> PowerLawFit {
    let above: Vec<u64> = values.iter().copied().filter(|v| *v >= x_min).collect();
    if above.len() < 2 || x_min == 0 {
        return PowerLawFit { alpha: 0.0, n: above.len() };
    }
    let xmin_shift = x_min as f64 - 0.5;
    let mut sum_ln = 0.0;
    for v in &above {
        sum_ln += (*v as f64 / xmin_shift).ln();
    }
    if sum_ln <= 0.0 {
        return PowerLawFit { alpha: 0.0, n: above.len() };
    }
    let alpha = 1.0 + above.len() as f64 / sum_ln;
    PowerLawFit { alpha, n: above.len() }
}

/// Two-sample Kolmogorov–Smirnov distance: the max abs difference
/// between the two empirical CDFs over the union of observed values.
/// Empty inputs return 0.0 (no distinguishable mass).
fn ks_distance(a: &[f64], b: &[f64]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut a_sorted: Vec<f64> = a.iter().copied().collect();
    let mut b_sorted: Vec<f64> = b.iter().copied().collect();
    a_sorted.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    b_sorted.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));

    let mut points: Vec<f64> = a_sorted.iter().chain(b_sorted.iter()).copied().collect();
    points.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    points.dedup_by(|x, y| (*x - *y).abs() < f64::EPSILON);

    let cdf = |sorted: &[f64], t: f64| -> f64 {
        let mut hi = sorted.len();
        let mut lo = 0;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if sorted[mid] <= t {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo as f64 / sorted.len() as f64
    };

    let mut d = 0.0_f64;
    for p in &points {
        let ca = cdf(&a_sorted, *p);
        let cb = cdf(&b_sorted, *p);
        d = d.max((ca - cb).abs());
    }
    d
}

fn posix(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tarjan_returns_one_scc_per_connected_cycle() {
        // 0 → 1 → 2 → 0, plus an isolated node 3.
        let edges = vec![(0, 1), (1, 2), (2, 0)];
        let sccs = tarjan_scc(4, &edges);
        // Three nodes in one SCC, and {3} as a singleton.
        let sizes: Vec<usize> = sccs.iter().map(|s| s.len()).collect();
        let mut sorted = sizes.clone();
        sorted.sort();
        assert_eq!(sorted, vec![1, 3]);
        // The 3-cycle SCC contains {0,1,2}.
        let big = sccs.iter().find(|s| s.len() == 3).unwrap();
        assert_eq!(big, &vec![0, 1, 2]);
    }

    #[test]
    fn tarjan_chain_yields_singleton_sccs() {
        let edges = vec![(0, 1), (1, 2), (2, 3)];
        let sccs = tarjan_scc(4, &edges);
        assert_eq!(sccs.len(), 4);
        for s in &sccs {
            assert_eq!(s.len(), 1);
        }
    }

    #[test]
    fn modularity_q_zero_on_empty_graph() {
        assert_eq!(modularity_q(3, &[], &[0, 1, 2]), 0.0);
    }

    #[test]
    fn modularity_q_positive_for_two_disjoint_cliques() {
        // Two triangles {0,1,2} and {3,4,5}, no bridge → high modularity.
        let edges = vec![
            (0, 1), (1, 2), (2, 0),
            (3, 4), (4, 5), (5, 3),
        ];
        let label = vec![0, 0, 0, 1, 1, 1];
        let q = modularity_q(6, &edges, &label);
        // Theoretical for two cliques: Q close to 0.5.
        assert!(q > 0.3, "expected Q > 0.3, got {q}");
    }

    #[test]
    fn label_propagation_groups_two_cliques() {
        let edges = vec![
            (0, 1), (1, 2), (2, 0),
            (3, 4), (4, 5), (5, 3),
        ];
        let (partition, q) = label_propagation(6, &edges);
        // Same partition for each triangle.
        assert_eq!(partition[0], partition[1]);
        assert_eq!(partition[1], partition[2]);
        assert_eq!(partition[3], partition[4]);
        assert_eq!(partition[4], partition[5]);
        // The triangles are different communities.
        assert_ne!(partition[0], partition[3]);
        assert!(q > 0.3, "expected Q > 0.3 on two cliques, got {q}");
    }

    #[test]
    fn ks_distance_zero_on_identical_distributions() {
        let a = vec![1.0, 2.0, 3.0];
        let d = ks_distance(&a, &a);
        assert!(d < 1e-9, "KS distance for identical distributions should be 0, got {d}");
    }

    #[test]
    fn ks_distance_handles_empty_inputs() {
        assert_eq!(ks_distance(&[], &[1.0]), 0.0);
        assert_eq!(ks_distance(&[1.0], &[]), 0.0);
    }

    #[test]
    fn power_law_mle_returns_zero_for_unfittable() {
        let fit = power_law_mle(&[], 1);
        assert_eq!(fit.alpha, 0.0);
        assert_eq!(fit.n, 0);
        // Single sample is also not fittable.
        let fit1 = power_law_mle(&[5], 1);
        assert_eq!(fit1.alpha, 0.0);
    }

    #[test]
    fn power_law_mle_recovers_a_reasonable_exponent_on_skewed_sample() {
        // Mock a Pareto-like sample (1 appears many times, then a long
        // tail). Expect alpha > 1, conventionally in the 1.5–3 range.
        let mut sample = vec![1u64; 50];
        sample.extend([2, 2, 2, 3, 3, 5, 8, 13]);
        let fit = power_law_mle(&sample, 1);
        assert!(fit.alpha > 1.5, "expected alpha > 1.5, got {}", fit.alpha);
        assert!(fit.alpha < 6.0, "alpha shouldn't blow up: {}", fit.alpha);
    }
}
