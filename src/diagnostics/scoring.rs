use std::path::{Path, PathBuf};

use crate::circuits;
use crate::complexity;
use crate::deps;
use crate::flow;
use crate::loc;
use crate::redundancy;

use super::{HealthScores, COGNITIVE, CYCLOMATIC, FILE_LOC, REDUNDANCY};

pub(super) fn compute_health_scores(
    file_stats: &[loc::FileLocStats],
    func_complexities: &[complexity::FunctionComplexity],
    dep_analysis: &deps::DepAnalysis,
    redundant_pairs: &[redundancy::RedundantPair],
    call_graph: &flow::CallGraph,
    reachable: &std::collections::HashSet<String>,
    rs_files: &[PathBuf],
    project_path: &Path,
) -> HealthScores {
    let modularity = compute_modularity_score(dep_analysis, rs_files, project_path);
    let complexity = compute_complexity_score(func_complexities);
    let maintainability = compute_maintainability_score(
        file_stats,
        func_complexities,
        redundant_pairs,
        call_graph,
        reachable,
        modularity,
        complexity,
    );

    HealthScores {
        modularity,
        complexity,
        maintainability,
    }
}

fn compute_modularity_score(
    dep_analysis: &deps::DepAnalysis,
    rs_files: &[PathBuf],
    project_path: &Path,
) -> f64 {
    if dep_analysis.modules.is_empty() {
        return 1.0;
    }

    let bidi_count = dep_analysis
        .coupling_pairs
        .iter()
        .filter(|(_, _, s)| *s >= 2)
        .count();
    let total_pairs = dep_analysis.coupling_pairs.len().max(1);
    let coupling_ratio = bidi_count as f64 / total_pairs as f64;
    let coupling_score = 1.0 - coupling_ratio;

    let modularity_q = circuits::circuit_summary(rs_files, project_path)
        .map(|cs| cs.modularity)
        .unwrap_or(0.0);
    let q_score = ((modularity_q + 0.5) / 1.5).clamp(0.0, 1.0);

    (0.5 * coupling_score + 0.5 * q_score).clamp(0.0, 1.0)
}

fn compute_complexity_score(func_complexities: &[complexity::FunctionComplexity]) -> f64 {
    if func_complexities.is_empty() {
        return 1.0;
    }

    let clean_cycl = func_complexities
        .iter()
        .filter(|fc| (fc.cyclomatic as f64) <= CYCLOMATIC.warn)
        .count();
    let cycl_ratio = clean_cycl as f64 / func_complexities.len() as f64;

    let clean_cogn = func_complexities
        .iter()
        .filter(|fc| (fc.cognitive as f64) <= COGNITIVE.warn)
        .count();
    let cogn_ratio = clean_cogn as f64 / func_complexities.len() as f64;

    let mut scores: Vec<f64> = func_complexities
        .iter()
        .map(|fc| fc.composite_score)
        .collect();
    scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = if scores.len() % 2 == 0 {
        (scores[scores.len() / 2 - 1] + scores[scores.len() / 2]) / 2.0
    } else {
        scores[scores.len() / 2]
    };
    let median_score = (1.0 - (median / 30.0)).clamp(0.0, 1.0);

    (0.4 * cycl_ratio + 0.3 * cogn_ratio + 0.3 * median_score).clamp(0.0, 1.0)
}

fn compute_maintainability_score(
    file_stats: &[loc::FileLocStats],
    func_complexities: &[complexity::FunctionComplexity],
    redundant_pairs: &[redundancy::RedundantPair],
    call_graph: &flow::CallGraph,
    reachable: &std::collections::HashSet<String>,
    modularity: f64,
    complexity: f64,
) -> f64 {
    let clean_files = file_stats
        .iter()
        .filter(|f| (f.code_lines as f64) <= FILE_LOC.warn)
        .count();
    let file_ratio = if file_stats.is_empty() {
        1.0
    } else {
        clean_files as f64 / file_stats.len() as f64
    };

    let high_redundancy = redundant_pairs
        .iter()
        .filter(|p| p.similarity > REDUNDANCY.alert)
        .count();
    let total_funcs = func_complexities.len().max(1);
    let redundancy_ratio =
        (1.0 - (high_redundancy as f64 * 2.0 / total_funcs as f64)).clamp(0.0, 1.0);

    let dead_ratio = if call_graph.functions.is_empty() {
        1.0
    } else {
        let dead = call_graph
            .functions
            .iter()
            .filter(|f| !reachable.contains(*f))
            .count();
        (1.0 - dead as f64 / call_graph.functions.len() as f64).clamp(0.0, 1.0)
    };

    (0.25 * modularity
        + 0.25 * complexity
        + 0.2 * file_ratio
        + 0.15 * redundancy_ratio
        + 0.15 * dead_ratio)
        .clamp(0.0, 1.0)
}

// ── Simple Tarjan SCC (to avoid depending on graph module internals) ─

pub(super) fn tarjan_scc_simple(
    nodes: &[String],
    adj: &std::collections::HashMap<String, Vec<String>>,
) -> Vec<Vec<String>> {
    struct State {
        index_counter: usize,
        stack: Vec<String>,
        on_stack: std::collections::HashSet<String>,
        index: std::collections::HashMap<String, usize>,
        lowlink: std::collections::HashMap<String, usize>,
        sccs: Vec<Vec<String>>,
    }

    fn strongconnect(
        v: &str,
        adj: &std::collections::HashMap<String, Vec<String>>,
        state: &mut State,
    ) {
        state.index.insert(v.to_string(), state.index_counter);
        state.lowlink.insert(v.to_string(), state.index_counter);
        state.index_counter += 1;
        state.stack.push(v.to_string());
        state.on_stack.insert(v.to_string());

        if let Some(neighbors) = adj.get(v) {
            for w in neighbors {
                if !state.index.contains_key(w.as_str()) {
                    strongconnect(w, adj, state);
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

    let mut state = State {
        index_counter: 0,
        stack: Vec::new(),
        on_stack: std::collections::HashSet::new(),
        index: std::collections::HashMap::new(),
        lowlink: std::collections::HashMap::new(),
        sccs: Vec::new(),
    };

    for node in nodes {
        if !state.index.contains_key(node.as_str()) {
            strongconnect(node, adj, &mut state);
        }
    }

    state.sccs
}
