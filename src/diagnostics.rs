use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::ast_parser;
use crate::circuits;
use crate::complexity;
use crate::deps;
use crate::flow;
use crate::loc;
use crate::redundancy;

// ── Diagnostic types ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub category: Category,
    pub location: Location,
    pub metric: f64,
    pub threshold: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Warn,
    Alert,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    BloatedFile,
    BloatedFunction,
    HighComplexity,
    HighCognitive,
    DeepNesting,
    HighCoupling,
    GodModule,
    DeadCode,
    LowCohesion,
    RedundantCode,
    TooManyParams,
    CyclicDeps,
}

#[derive(Debug, Clone, Serialize)]
pub struct Location {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_end: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthScores {
    pub modularity: f64,
    pub complexity: f64,
    pub maintainability: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticReport {
    pub diagnostics: Vec<Diagnostic>,
    pub scores: HealthScores,
}

// ── Thresholds ───────────────────────────────────────────────────

struct Threshold {
    warn: f64,
    alert: f64,
}

const FILE_LOC: Threshold = Threshold { warn: 500.0, alert: 800.0 };
const FUNC_LOC: Threshold = Threshold { warn: 50.0, alert: 100.0 };
const CYCLOMATIC: Threshold = Threshold { warn: 10.0, alert: 20.0 };
const COGNITIVE: Threshold = Threshold { warn: 15.0, alert: 30.0 };
const NESTING: Threshold = Threshold { warn: 4.0, alert: 6.0 };
const PARAMS: Threshold = Threshold { warn: 5.0, alert: 8.0 };
const REDUNDANCY: Threshold = Threshold { warn: 0.7, alert: 0.9 };
const COHESION: Threshold = Threshold { warn: 0.3, alert: 0.1 };

// ── Main analysis entry point ────────────────────────────────────

pub fn run_diagnostics(rs_files: &[PathBuf], project_path: &Path) -> DiagnosticReport {
    let mut diagnostics = Vec::new();

    // --- LoC analysis ---
    let (file_stats, _agg_loc) = loc::analyze_files(rs_files);

    // --- Complexity analysis ---
    let symbols = ast_parser::parse_project(rs_files);
    let func_complexities = complexity::compute_all(&symbols, project_path);

    // Build per-file function index for cross-referencing
    let mut funcs_by_file: std::collections::HashMap<String, Vec<&complexity::FunctionComplexity>> =
        std::collections::HashMap::new();
    for fc in &func_complexities {
        funcs_by_file.entry(fc.file.clone()).or_default().push(fc);
    }

    // --- Bloated file diagnostics (with context) ---
    for stat in &file_stats {
        let loc = stat.code_lines as f64;
        let rel_path = stat
            .path
            .strip_prefix(project_path)
            .unwrap_or(&stat.path)
            .display()
            .to_string();

        let (sev, thresh) = if loc > FILE_LOC.alert {
            (Some(Severity::Alert), FILE_LOC.alert)
        } else if loc > FILE_LOC.warn {
            (Some(Severity::Warn), FILE_LOC.warn)
        } else {
            (None, 0.0)
        };

        if let Some(severity) = sev {
            let file_funcs = funcs_by_file.get(rel_path.as_str());
            let func_count = file_funcs.map(|v| v.len()).unwrap_or(0);
            let top_complex: Vec<serde_json::Value> = file_funcs
                .map(|fns| {
                    let mut sorted: Vec<_> = fns.iter().collect();
                    sorted.sort_by(|a, b| b.composite_score.partial_cmp(&a.composite_score).unwrap_or(std::cmp::Ordering::Equal));
                    sorted.into_iter().take(3).map(|f| {
                        serde_json::json!({
                            "name": f.name,
                            "composite_score": f.composite_score,
                            "lines": f.line_count
                        })
                    }).collect()
                })
                .unwrap_or_default();

            let ctx = serde_json::json!({
                "function_count": func_count,
                "top_complex_functions": top_complex,
            });

            let msg = if severity == Severity::Alert {
                format!("{} is {} lines with {} functions", rel_path, stat.code_lines, func_count)
            } else {
                format!("{} is {} lines with {} functions", rel_path, stat.code_lines, func_count)
            };

            diagnostics.push(Diagnostic {
                severity,
                category: Category::BloatedFile,
                location: Location {
                    file: Some(rel_path.clone()),
                    function: None,
                    module: None,
                    line_start: None,
                    line_end: None,
                },
                metric: loc,
                threshold: thresh,
                context: Some(ctx),
                message: msg,
            });
        }
    }

    // --- Function-level diagnostics (with cross-referenced context) ---
    for fc in &func_complexities {
        let line_count = fc.line_count as f64;
        let cyc = fc.cyclomatic as f64;
        let cog = fc.cognitive as f64;
        let nest = fc.nesting_depth as f64;
        let params = fc.param_count as f64;

        // Bloated function
        if let Some((sev, thresh)) = check_threshold(line_count, &FUNC_LOC) {
            let ctx = serde_json::json!({
                "cyclomatic": fc.cyclomatic,
                "cognitive": fc.cognitive,
                "nesting_depth": fc.nesting_depth,
                "composite_score": fc.composite_score,
            });
            let msg = format!(
                "{} is {} lines (cyclomatic {}, cognitive {})",
                fc.name, fc.line_count, fc.cyclomatic, fc.cognitive
            );
            diagnostics.push(Diagnostic {
                severity: sev,
                category: Category::BloatedFunction,
                location: fn_location(&fc.name, &fc.file),
                metric: line_count,
                threshold: thresh,
                context: Some(ctx),
                message: msg,
            });
        }

        // Cyclomatic complexity
        if let Some((sev, thresh)) = check_threshold(cyc, &CYCLOMATIC) {
            let ctx = serde_json::json!({
                "cognitive": fc.cognitive,
                "nesting_depth": fc.nesting_depth,
                "line_count": fc.line_count,
            });
            let msg = format!(
                "{} has cyclomatic complexity {} ({} lines, nesting depth {})",
                fc.name, fc.cyclomatic, fc.line_count, fc.nesting_depth
            );
            diagnostics.push(Diagnostic {
                severity: sev,
                category: Category::HighComplexity,
                location: fn_location(&fc.name, &fc.file),
                metric: cyc,
                threshold: thresh,
                context: Some(ctx),
                message: msg,
            });
        }

        // Cognitive complexity
        if let Some((sev, thresh)) = check_threshold(cog, &COGNITIVE) {
            let ctx = serde_json::json!({
                "cyclomatic": fc.cyclomatic,
                "nesting_depth": fc.nesting_depth,
                "line_count": fc.line_count,
            });
            let msg = format!(
                "{} has cognitive complexity {} (cyclomatic {}, nesting depth {})",
                fc.name, fc.cognitive, fc.cyclomatic, fc.nesting_depth
            );
            diagnostics.push(Diagnostic {
                severity: sev,
                category: Category::HighCognitive,
                location: fn_location(&fc.name, &fc.file),
                metric: cog,
                threshold: thresh,
                context: Some(ctx),
                message: msg,
            });
        }

        // Deep nesting
        if let Some((sev, thresh)) = check_threshold(nest, &NESTING) {
            let ctx = serde_json::json!({
                "cyclomatic": fc.cyclomatic,
                "cognitive": fc.cognitive,
                "line_count": fc.line_count,
            });
            let msg = format!(
                "{} has nesting depth {} (cyclomatic {}, {} lines)",
                fc.name, fc.nesting_depth, fc.cyclomatic, fc.line_count
            );
            diagnostics.push(Diagnostic {
                severity: sev,
                category: Category::DeepNesting,
                location: fn_location(&fc.name, &fc.file),
                metric: nest,
                threshold: thresh,
                context: Some(ctx),
                message: msg,
            });
        }

        // Too many params
        if let Some((sev, thresh)) = check_threshold(params, &PARAMS) {
            let ctx = serde_json::json!({
                "line_count": fc.line_count,
                "cyclomatic": fc.cyclomatic,
            });
            let msg = format!(
                "{} takes {} parameters ({} lines, cyclomatic {})",
                fc.name, fc.param_count, fc.line_count, fc.cyclomatic
            );
            diagnostics.push(Diagnostic {
                severity: sev,
                category: Category::TooManyParams,
                location: fn_location(&fc.name, &fc.file),
                metric: params,
                threshold: thresh,
                context: Some(ctx),
                message: msg,
            });
        }
    }

    // --- Dependency analysis ---
    let dep_analysis = deps::analyze_deps(rs_files, project_path);

    // High coupling (bidirectional dependencies)
    for (a, b, strength) in &dep_analysis.coupling_pairs {
        if *strength >= 2 {
            let fan_in_a = dep_analysis.in_degree.get(a).copied().unwrap_or(0);
            let fan_out_a = dep_analysis.out_degree.get(a).copied().unwrap_or(0);
            let fan_in_b = dep_analysis.in_degree.get(b).copied().unwrap_or(0);
            let fan_out_b = dep_analysis.out_degree.get(b).copied().unwrap_or(0);
            let ctx = serde_json::json!({
                "module_a": { "fan_in": fan_in_a, "fan_out": fan_out_a },
                "module_b": { "fan_in": fan_in_b, "fan_out": fan_out_b },
            });
            diagnostics.push(Diagnostic {
                severity: Severity::Warn,
                category: Category::HighCoupling,
                location: Location {
                    file: None,
                    function: None,
                    module: Some(format!("{} <-> {}", a, b)),
                    line_start: None,
                    line_end: None,
                },
                metric: *strength as f64,
                threshold: 2.0,
                context: Some(ctx),
                message: format!(
                    "{} and {} have bidirectional dependency (strength {})",
                    a, b, strength
                ),
            });
        }
    }

    // God module detection (fan-in + fan-out > mean + 1σ)
    let fan_totals: Vec<f64> = dep_analysis
        .modules
        .iter()
        .map(|m| {
            let in_d = dep_analysis.in_degree.get(m).copied().unwrap_or(0);
            let out_d = dep_analysis.out_degree.get(m).copied().unwrap_or(0);
            (in_d + out_d) as f64
        })
        .collect();

    if !fan_totals.is_empty() {
        let mean_fan = fan_totals.iter().sum::<f64>() / fan_totals.len() as f64;
        let var_fan = fan_totals.iter().map(|v| (v - mean_fan).powi(2)).sum::<f64>()
            / fan_totals.len() as f64;
        let std_fan = var_fan.sqrt();
        let warn_thresh = mean_fan + std_fan;
        let alert_thresh = mean_fan + 2.0 * std_fan;

        for (i, m) in dep_analysis.modules.iter().enumerate() {
            let total = fan_totals[i];
            let in_d = dep_analysis.in_degree.get(m).copied().unwrap_or(0);
            let out_d = dep_analysis.out_degree.get(m).copied().unwrap_or(0);

            if total > alert_thresh && alert_thresh > 0.0 {
                let ctx = serde_json::json!({
                    "fan_in": in_d,
                    "fan_out": out_d,
                    "mean": mean_fan,
                    "stddev": std_fan,
                });
                diagnostics.push(Diagnostic {
                    severity: Severity::Alert,
                    category: Category::GodModule,
                    location: mod_location(m),
                    metric: total,
                    threshold: alert_thresh,
                    context: Some(ctx),
                    message: format!(
                        "{} has fan-in+fan-out of {} (mean+2σ={:.1}, fan-in {}, fan-out {})",
                        m, total as usize, alert_thresh, in_d, out_d
                    ),
                });
            } else if total > warn_thresh && warn_thresh > 0.0 {
                let ctx = serde_json::json!({
                    "fan_in": in_d,
                    "fan_out": out_d,
                    "mean": mean_fan,
                    "stddev": std_fan,
                });
                diagnostics.push(Diagnostic {
                    severity: Severity::Warn,
                    category: Category::GodModule,
                    location: mod_location(m),
                    metric: total,
                    threshold: warn_thresh,
                    context: Some(ctx),
                    message: format!(
                        "{} has fan-in+fan-out of {} (mean+1σ={:.1}, fan-in {}, fan-out {})",
                        m, total as usize, warn_thresh, in_d, out_d
                    ),
                });
            }
        }
    }

    // --- Dead code (cold functions) ---
    let call_graph = flow::build_call_graph(rs_files, project_path);
    let reachable = {
        let mut reachable = std::collections::HashSet::new();
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
    };

    for func in &call_graph.functions {
        if !reachable.contains(func) {
            diagnostics.push(Diagnostic {
                severity: Severity::Warn,
                category: Category::DeadCode,
                location: Location {
                    file: None,
                    function: Some(func.clone()),
                    module: None,
                    line_start: None,
                    line_end: None,
                },
                metric: 0.0,
                threshold: 0.0,
                context: None,
                message: format!(
                    "{} is not reachable from any entry point",
                    func
                ),
            });
        }
    }

    // --- Cyclic dependencies (SCCs with > 1 member in module dep graph) ---
    {
        let mut adj: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for m in &dep_analysis.modules {
            adj.entry(m.clone()).or_default();
        }
        for edge in &dep_analysis.edges {
            adj.entry(edge.from.clone()).or_default().push(edge.to.clone());
        }

        let sccs = tarjan_scc_simple(&dep_analysis.modules, &adj);
        for scc in &sccs {
            if scc.len() > 1 {
                let members = scc.join(", ");
                let ctx = serde_json::json!({
                    "members": scc,
                    "member_count": scc.len(),
                });
                diagnostics.push(Diagnostic {
                    severity: Severity::Warn,
                    category: Category::CyclicDeps,
                    location: Location {
                        file: None,
                        function: None,
                        module: Some(members.clone()),
                        line_start: None,
                        line_end: None,
                    },
                    metric: scc.len() as f64,
                    threshold: 1.0,
                    context: Some(ctx),
                    message: format!(
                        "Cyclic dependency among {} modules: {}",
                        scc.len(),
                        members
                    ),
                });
            }
        }
    }

    // --- Redundancy ---
    let redundant_pairs = redundancy::detect_redundancy(rs_files, project_path);
    for pair in &redundant_pairs {
        let (sev, thresh) = if pair.similarity > REDUNDANCY.alert {
            (Some(Severity::Alert), REDUNDANCY.alert)
        } else if pair.similarity > REDUNDANCY.warn {
            (Some(Severity::Warn), REDUNDANCY.warn)
        } else {
            (None, 0.0)
        };

        if let Some(severity) = sev {
            let ctx = serde_json::json!({
                "func_a": pair.func_a,
                "func_b": pair.func_b,
                "file_a": pair.file_a,
                "file_b": pair.file_b,
                "kind": pair.kind.to_string(),
            });
            let msg = format!(
                "{} and {} have {:.0}% similarity ({})",
                pair.func_a, pair.func_b, pair.similarity * 100.0, pair.kind
            );
            diagnostics.push(Diagnostic {
                severity,
                category: Category::RedundantCode,
                location: Location {
                    file: Some(pair.file_a.clone()),
                    function: Some(format!("{} / {}", pair.func_a, pair.func_b)),
                    module: None,
                    line_start: Some(pair.line_a),
                    line_end: None,
                },
                metric: pair.similarity,
                threshold: thresh,
                context: Some(ctx),
                message: msg,
            });
        }
    }

    // --- Low cohesion ---
    let cohesion_data = deps::compute_cohesion(rs_files, project_path);
    for mc in &cohesion_data {
        if mc.function_count <= 1 {
            continue; // singleton modules are trivially cohesive
        }
        let (sev, thresh) = if mc.combined < COHESION.alert {
            (Some(Severity::Alert), COHESION.alert)
        } else if mc.combined < COHESION.warn {
            (Some(Severity::Warn), COHESION.warn)
        } else {
            (None, 0.0)
        };

        if let Some(severity) = sev {
            let ctx = serde_json::json!({
                "call_cohesion": mc.call_cohesion,
                "type_cohesion": mc.type_cohesion,
                "function_count": mc.function_count,
            });
            let msg = format!(
                "{} has cohesion {:.2} (call {:.2}, type {:.2}, {} functions)",
                mc.module, mc.combined, mc.call_cohesion, mc.type_cohesion, mc.function_count
            );
            diagnostics.push(Diagnostic {
                severity,
                category: Category::LowCohesion,
                location: mod_location(&mc.module),
                metric: mc.combined,
                threshold: thresh,
                context: Some(ctx),
                message: msg,
            });
        }
    }

    // --- Sort diagnostics: alerts first, then by metric magnitude descending ---
    diagnostics.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| {
                b.metric
                    .partial_cmp(&a.metric)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    // --- Compute health scores ---
    let scores = compute_health_scores(
        &file_stats,
        &func_complexities,
        &dep_analysis,
        &redundant_pairs,
        &call_graph,
        &reachable,
        rs_files,
        project_path,
    );

    DiagnosticReport {
        diagnostics,
        scores,
    }
}

// ── Health score computation ─────────────────────────────────────

fn compute_health_scores(
    file_stats: &[loc::FileLocStats],
    func_complexities: &[complexity::FunctionComplexity],
    dep_analysis: &deps::DepAnalysis,
    redundant_pairs: &[redundancy::RedundantPair],
    call_graph: &flow::CallGraph,
    reachable: &std::collections::HashSet<String>,
    rs_files: &[PathBuf],
    project_path: &Path,
) -> HealthScores {
    // Modularity score (0.0-1.0, higher = healthier)
    let modularity = compute_modularity_score(dep_analysis, rs_files, project_path);

    // Complexity score (0.0-1.0, higher = healthier)
    let complexity = compute_complexity_score(func_complexities);

    // Maintainability score (composite)
    let maintainability =
        compute_maintainability_score(file_stats, func_complexities, redundant_pairs, call_graph, reachable, modularity, complexity);

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

    // Factor 1: bidirectional coupling ratio (lower = better)
    let bidi_count = dep_analysis
        .coupling_pairs
        .iter()
        .filter(|(_, _, s)| *s >= 2)
        .count();
    let total_pairs = dep_analysis.coupling_pairs.len().max(1);
    let coupling_ratio = bidi_count as f64 / total_pairs as f64;
    let coupling_score = 1.0 - coupling_ratio;

    // Factor 2: modularity Q from circuit detection
    let modularity_q = circuits::circuit_summary(rs_files, project_path)
        .map(|cs| cs.modularity)
        .unwrap_or(0.0);
    // Q ranges roughly -0.5 to 1.0, normalize to 0.0-1.0
    let q_score = ((modularity_q + 0.5) / 1.5).clamp(0.0, 1.0);

    // Weighted combination
    (0.5 * coupling_score + 0.5 * q_score).clamp(0.0, 1.0)
}

fn compute_complexity_score(func_complexities: &[complexity::FunctionComplexity]) -> f64 {
    if func_complexities.is_empty() {
        return 1.0;
    }

    // Factor 1: what fraction of functions are below the warn threshold for cyclomatic?
    let clean_cycl = func_complexities
        .iter()
        .filter(|fc| (fc.cyclomatic as f64) <= CYCLOMATIC.warn)
        .count();
    let cycl_ratio = clean_cycl as f64 / func_complexities.len() as f64;

    // Factor 2: what fraction are below cognitive warn?
    let clean_cogn = func_complexities
        .iter()
        .filter(|fc| (fc.cognitive as f64) <= COGNITIVE.warn)
        .count();
    let cogn_ratio = clean_cogn as f64 / func_complexities.len() as f64;

    // Factor 3: median complexity relative to warn threshold
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
    // If median composite < 15 that's good, > 30 is bad
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
    // Factor 1: file size distribution (fraction of files below warn threshold)
    let clean_files = file_stats
        .iter()
        .filter(|f| (f.code_lines as f64) <= FILE_LOC.warn)
        .count();
    let file_ratio = if file_stats.is_empty() {
        1.0
    } else {
        clean_files as f64 / file_stats.len() as f64
    };

    // Factor 2: redundancy penalty
    let high_redundancy = redundant_pairs
        .iter()
        .filter(|p| p.similarity > REDUNDANCY.alert)
        .count();
    let total_funcs = func_complexities.len().max(1);
    let redundancy_ratio = (1.0 - (high_redundancy as f64 * 2.0 / total_funcs as f64)).clamp(0.0, 1.0);

    // Factor 3: dead code ratio
    let dead_ratio = if call_graph.functions.is_empty() {
        1.0
    } else {
        let dead = call_graph.functions.iter().filter(|f| !reachable.contains(*f)).count();
        (1.0 - dead as f64 / call_graph.functions.len() as f64).clamp(0.0, 1.0)
    };

    // Composite: weighted average including modularity and complexity
    (0.25 * modularity + 0.25 * complexity + 0.2 * file_ratio + 0.15 * redundancy_ratio + 0.15 * dead_ratio)
        .clamp(0.0, 1.0)
}

// ── Simple Tarjan SCC (to avoid depending on graph module internals) ─

fn tarjan_scc_simple(
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

// ── Helpers ──────────────────────────────────────────────────────

fn check_threshold(value: f64, thresh: &Threshold) -> Option<(Severity, f64)> {
    if value > thresh.alert {
        Some((Severity::Alert, thresh.alert))
    } else if value > thresh.warn {
        Some((Severity::Warn, thresh.warn))
    } else {
        None
    }
}

fn fn_location(name: &str, file: &str) -> Location {
    Location {
        file: Some(file.to_string()),
        function: Some(name.to_string()),
        module: None,
        line_start: None,
        line_end: None,
    }
}

fn mod_location(module: &str) -> Location {
    Location {
        file: None,
        function: None,
        module: Some(module.to_string()),
        line_start: None,
        line_end: None,
    }
}
