mod scoring;

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast_parser;
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

pub(crate) struct Threshold {
    pub(crate) warn: f64,
    pub(crate) alert: f64,
}

pub(crate) const FILE_LOC: Threshold = Threshold {
    warn: 500.0,
    alert: 800.0,
};
const FUNC_LOC: Threshold = Threshold {
    warn: 50.0,
    alert: 100.0,
};
pub(crate) const CYCLOMATIC: Threshold = Threshold {
    warn: 10.0,
    alert: 20.0,
};
pub(crate) const COGNITIVE: Threshold = Threshold {
    warn: 15.0,
    alert: 30.0,
};
const NESTING: Threshold = Threshold {
    warn: 4.0,
    alert: 6.0,
};
const PARAMS: Threshold = Threshold {
    warn: 5.0,
    alert: 8.0,
};
pub(crate) const REDUNDANCY: Threshold = Threshold {
    warn: 0.7,
    alert: 0.9,
};
pub(crate) const COHESION: Threshold = Threshold {
    warn: 0.3,
    alert: 0.1,
};

// ── Main analysis entry point (orchestrator) ─────────────────────

pub fn run_diagnostics(rs_files: &[PathBuf], project_path: &Path) -> DiagnosticReport {
    let mut diagnostics = Vec::new();

    // --- LoC analysis ---
    let (file_stats, _agg_loc) = loc::analyze_files(rs_files);

    // --- Complexity analysis ---
    let symbols = ast_parser::parse_project(rs_files);
    let func_complexities = complexity::compute_all(&symbols, project_path);

    // Build per-file function index for cross-referencing
    let mut funcs_by_file: HashMap<String, Vec<&complexity::FunctionComplexity>> = HashMap::new();
    for fc in &func_complexities {
        funcs_by_file.entry(fc.file.clone()).or_default().push(fc);
    }

    diagnose_bloated_files(&file_stats, &funcs_by_file, project_path, &mut diagnostics);
    diagnose_function_metrics(&func_complexities, &mut diagnostics);

    // --- Dependency analysis ---
    let dep_analysis = deps::analyze_deps(rs_files, project_path);

    diagnose_coupling(&dep_analysis, &mut diagnostics);
    diagnose_god_modules(&dep_analysis, &mut diagnostics);

    // --- Dead code (cold functions) ---
    let call_graph = flow::build_call_graph(rs_files, project_path);
    let reachable = compute_reachable(&call_graph);
    diagnose_dead_code(&call_graph, &reachable, &mut diagnostics);

    diagnose_cyclic_deps(&dep_analysis, &mut diagnostics);

    // --- Redundancy ---
    let redundant_pairs = redundancy::detect_redundancy(rs_files, project_path);
    diagnose_redundancy(&redundant_pairs, &mut diagnostics);

    // --- Low cohesion ---
    let cohesion_data = deps::compute_cohesion(rs_files, project_path);
    diagnose_low_cohesion(&cohesion_data, &mut diagnostics);

    // --- Sort diagnostics: alerts first, then by metric magnitude descending ---
    diagnostics.sort_by(|a, b| {
        a.severity.cmp(&b.severity).then_with(|| {
            b.metric
                .partial_cmp(&a.metric)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    // --- Compute health scores ---
    let scores = scoring::compute_health_scores(
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

// ── Extracted diagnostic helpers ────────────────────────────────

fn diagnose_bloated_files(
    file_stats: &[loc::FileLocStats],
    funcs_by_file: &HashMap<String, Vec<&complexity::FunctionComplexity>>,
    project_path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stat in file_stats {
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
                    sorted.sort_by(|a, b| {
                        b.composite_score
                            .partial_cmp(&a.composite_score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                    sorted
                        .into_iter()
                        .take(3)
                        .map(|f| {
                            serde_json::json!({
                                "name": f.name,
                                "composite_score": f.composite_score,
                                "lines": f.line_count
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();

            let ctx = serde_json::json!({
                "function_count": func_count,
                "top_complex_functions": top_complex,
            });

            let msg = format!(
                "{} is {} lines with {} functions",
                rel_path, stat.code_lines, func_count
            );

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
}

fn diagnose_function_metrics(
    func_complexities: &[complexity::FunctionComplexity],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for fc in func_complexities {
        diagnose_fn_complexity(fc, diagnostics);
        diagnose_fn_structure(fc, diagnostics);
    }
}

fn diagnose_fn_complexity(fc: &complexity::FunctionComplexity, diagnostics: &mut Vec<Diagnostic>) {
    let line_count = fc.line_count as f64;
    let cyc = fc.cyclomatic as f64;
    let cog = fc.cognitive as f64;

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
}

fn diagnose_fn_structure(fc: &complexity::FunctionComplexity, diagnostics: &mut Vec<Diagnostic>) {
    let nest = fc.nesting_depth as f64;
    let params = fc.param_count as f64;

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

fn diagnose_coupling(dep_analysis: &deps::DepAnalysis, diagnostics: &mut Vec<Diagnostic>) {
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
}

fn diagnose_god_modules(dep_analysis: &deps::DepAnalysis, diagnostics: &mut Vec<Diagnostic>) {
    let fan_totals: Vec<f64> = dep_analysis
        .modules
        .iter()
        .map(|m| {
            let in_d = dep_analysis.in_degree.get(m).copied().unwrap_or(0);
            let out_d = dep_analysis.out_degree.get(m).copied().unwrap_or(0);
            (in_d + out_d) as f64
        })
        .collect();

    if fan_totals.is_empty() {
        return;
    }

    let mean_fan = fan_totals.iter().sum::<f64>() / fan_totals.len() as f64;
    let var_fan = fan_totals
        .iter()
        .map(|v| (v - mean_fan).powi(2))
        .sum::<f64>()
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

fn compute_reachable(call_graph: &flow::CallGraph) -> HashSet<String> {
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

fn diagnose_dead_code(
    call_graph: &flow::CallGraph,
    reachable: &HashSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
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
                message: format!("{} is not reachable from any entry point", func),
            });
        }
    }
}

fn diagnose_cyclic_deps(dep_analysis: &deps::DepAnalysis, diagnostics: &mut Vec<Diagnostic>) {
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for m in &dep_analysis.modules {
        adj.entry(m.clone()).or_default();
    }
    for edge in &dep_analysis.edges {
        adj.entry(edge.from.clone())
            .or_default()
            .push(edge.to.clone());
    }

    let sccs = scoring::tarjan_scc_simple(&dep_analysis.modules, &adj);
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
                message: format!("Cyclic dependency among {} modules: {}", scc.len(), members),
            });
        }
    }
}

fn diagnose_redundancy(
    redundant_pairs: &[redundancy::RedundantPair],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for pair in redundant_pairs {
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
                pair.func_a,
                pair.func_b,
                pair.similarity * 100.0,
                pair.kind
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
}

fn diagnose_low_cohesion(
    cohesion_data: &[deps::ModuleCohesion],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for mc in cohesion_data {
        if mc.function_count <= 1 {
            continue;
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

fn make_location(file: Option<&str>, function: Option<&str>, module: Option<&str>) -> Location {
    Location {
        file: file.map(str::to_string),
        function: function.map(str::to_string),
        module: module.map(str::to_string),
        line_start: None,
        line_end: None,
    }
}

fn fn_location(name: &str, file: &str) -> Location {
    make_location(Some(file), Some(name), None)
}

fn mod_location(module: &str) -> Location {
    make_location(None, None, Some(module))
}
