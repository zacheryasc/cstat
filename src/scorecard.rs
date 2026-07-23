use colored::Colorize;
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::{ast_parser, dead_code, deps, flow, loc, render};

const SCORE_VERSION: &str = "code_complexity_cost_v0";
const DEFAULT_TOP: usize = 20;

#[derive(Debug, Clone, Serialize)]
pub struct ScorecardReport {
    pub cstat_version: String,
    pub score_version: String,
    pub target: String,
    pub code_complexity_cost: f64,
    pub code_complexity_cost_per_kloc: f64,
    pub component_costs: ComponentCosts,
    pub scope_breakdown: BTreeMap<String, ScopeSummary>,
    pub top_contributors: Vec<TopContributor>,
    pub metadata: ScoreMetadata,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentCosts {
    pub function_complexity: FunctionComponent,
    pub file_concentration: FileComponent,
    pub module_coupling: ModuleComponent,
    pub abstraction_surface: AbstractionComponent,
    pub stale_surface: StaleComponent,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionComponent {
    pub cost: f64,
    pub functions_scored: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileComponent {
    pub cost: f64,
    pub files_scored: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleComponent {
    pub cost: f64,
    pub modules_scored: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct AbstractionComponent {
    pub cost: f64,
    pub symbols_scored: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct StaleComponent {
    pub cost: f64,
    pub candidate_count: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ScopeSummary {
    pub cost: f64,
    pub functions_scored: usize,
    pub files_scored: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopContributor {
    pub kind: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    pub cost: f64,
    pub reasons: BTreeMap<String, serde_json::Value>,
    pub component_costs: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScoreMetadata {
    pub rust_files: usize,
    pub code_lines: usize,
    pub parse_error_files: usize,
}

#[derive(Debug, Clone, Default)]
struct ScopeAccumulator {
    cost: f64,
    functions_scored: usize,
    files_scored: usize,
}

#[derive(Debug, Clone)]
struct Contributor {
    kind: String,
    scope: String,
    file: Option<String>,
    module: Option<String>,
    function: Option<String>,
    cost: f64,
    reasons: BTreeMap<String, serde_json::Value>,
    component_costs: BTreeMap<String, f64>,
}

#[derive(Debug, Clone)]
struct FunctionScoreInput {
    file: String,
    function: String,
    graph_name: String,
    scope: String,
    cyclomatic: usize,
    nesting_depth: usize,
    line_count: usize,
    body_stmt_count: usize,
    signature_score: usize,
}

pub fn render_scorecard(
    files: &[PathBuf],
    project_path: &Path,
    target: &str,
    top: Option<usize>,
    json_output: bool,
    verbose: bool,
) {
    let report = analyze_scorecard(files, project_path, target, top.unwrap_or(DEFAULT_TOP));
    if json_output {
        println!("{}", serde_json::to_string(&report).unwrap());
    } else {
        render_scorecard_human(&report, verbose);
    }
}

pub fn analyze_scorecard(
    files: &[PathBuf],
    project_path: &Path,
    target: &str,
    top: usize,
) -> ScorecardReport {
    let symbols = ast_parser::parse_project(files);
    let graph = flow::build_call_graph(files, project_path);
    let loc_by_file = collect_loc_by_file(files);
    let file_scopes = collect_file_scopes(files, project_path);
    let function_inputs = collect_function_inputs(&symbols, &graph, project_path, &file_scopes);

    let mut scope_totals = initial_scope_totals(files, &symbols, project_path, &file_scopes);
    let mut contributors = Vec::new();

    let function_complexity =
        score_functions(&function_inputs, &mut scope_totals, &mut contributors);
    let file_concentration = score_files(
        &symbols,
        &loc_by_file,
        project_path,
        &file_scopes,
        &mut scope_totals,
        &mut contributors,
    );
    let abstraction_surface = score_abstraction_surface(
        &symbols,
        project_path,
        &file_scopes,
        &mut scope_totals,
        &mut contributors,
    );
    let module_coupling = score_module_coupling(
        files,
        project_path,
        &file_scopes,
        &mut scope_totals,
        &mut contributors,
    );
    let stale_surface = score_stale_surface(
        &graph,
        &function_inputs,
        &mut scope_totals,
        &mut contributors,
    );

    contributors.sort_by(|a, b| {
        b.cost
            .partial_cmp(&a.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.module.cmp(&b.module))
            .then_with(|| a.function.cmp(&b.function))
    });
    contributors.truncate(top);

    let code_lines: usize = loc_by_file.values().map(|stats| stats.code_lines).sum();
    let total_cost = function_complexity.cost
        + file_concentration.cost
        + module_coupling.cost
        + abstraction_surface.cost
        + stale_surface.cost;
    let kloc = (code_lines as f64 / 1000.0).max(1.0);

    ScorecardReport {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        score_version: SCORE_VERSION.to_string(),
        target: target.to_string(),
        code_complexity_cost: round1(total_cost),
        code_complexity_cost_per_kloc: round1(total_cost / kloc),
        component_costs: ComponentCosts {
            function_complexity: FunctionComponent {
                cost: round1(function_complexity.cost),
                functions_scored: function_complexity.functions_scored,
            },
            file_concentration: FileComponent {
                cost: round1(file_concentration.cost),
                files_scored: file_concentration.files_scored,
            },
            module_coupling: ModuleComponent {
                cost: round1(module_coupling.cost),
                modules_scored: module_coupling.modules_scored,
            },
            abstraction_surface: AbstractionComponent {
                cost: round1(abstraction_surface.cost),
                symbols_scored: abstraction_surface.symbols_scored,
            },
            stale_surface: StaleComponent {
                cost: round1(stale_surface.cost),
                candidate_count: stale_surface.candidate_count,
            },
        },
        scope_breakdown: finalize_scope_totals(scope_totals),
        top_contributors: contributors
            .into_iter()
            .map(|contributor| TopContributor {
                kind: contributor.kind,
                scope: contributor.scope,
                file: contributor.file,
                module: contributor.module,
                function: contributor.function,
                cost: round1(contributor.cost),
                reasons: contributor.reasons,
                component_costs: contributor
                    .component_costs
                    .into_iter()
                    .map(|(key, value)| (key, round1(value)))
                    .collect(),
            })
            .collect(),
        metadata: ScoreMetadata {
            rust_files: files.len(),
            code_lines,
            parse_error_files: symbols.files.iter().filter(|file| file.parse_error).count(),
        },
    }
}

fn render_scorecard_human(report: &ScorecardReport, verbose: bool) {
    render::section_header("Code complexity score");
    println!("score version: {}", report.score_version.bold());
    println!("lower is cleaner; harness guards behavior separately");
    if verbose {
        render::verbose_block(&[
            "Pure structural cost; no tests, benchmark, coverage, reward, or behavior signals are included.",
            "All discovered Rust scopes contribute equally; scope breakdown is explanatory only.",
            "This command is a compact scorecard, not a bundle of every underlying probe.",
        ]);
    } else {
        println!();
    }

    println!("total cost: {:.1}", report.code_complexity_cost);
    println!("cost / KLOC: {:.1}", report.code_complexity_cost_per_kloc);
    println!();
    println!("components:");
    println!(
        "  {:<24} {:>8.1}",
        "function complexity:", report.component_costs.function_complexity.cost
    );
    println!(
        "  {:<24} {:>8.1}",
        "file concentration:", report.component_costs.file_concentration.cost
    );
    println!(
        "  {:<24} {:>8.1}",
        "module coupling:", report.component_costs.module_coupling.cost
    );
    println!(
        "  {:<24} {:>8.1}",
        "abstraction surface:", report.component_costs.abstraction_surface.cost
    );
    println!(
        "  {:<24} {:>8.1}",
        "stale surface:", report.component_costs.stale_surface.cost
    );

    println!();
    println!("top contributors:");
    if report.top_contributors.is_empty() {
        println!("  <none>");
    } else {
        for (index, contributor) in report.top_contributors.iter().enumerate() {
            println!(
                "  {:>2}. {:<19} {:<48} {:>8.1}",
                index + 1,
                contributor.kind,
                contributor_label(contributor),
                contributor.cost,
            );
        }
    }
}

fn contributor_label(contributor: &TopContributor) -> String {
    match contributor.kind.as_str() {
        "function" => match (&contributor.file, &contributor.function) {
            (Some(file), Some(function)) => format!("{file}::{function}"),
            _ => contributor
                .function
                .clone()
                .unwrap_or_else(|| "<unknown>".to_string()),
        },
        "file" | "abstraction_surface" => contributor
            .file
            .clone()
            .unwrap_or_else(|| "<unknown>".to_string()),
        "module" => contributor
            .module
            .clone()
            .unwrap_or_else(|| "<unknown>".to_string()),
        "stale_surface" => contributor
            .function
            .clone()
            .unwrap_or_else(|| "<unknown>".to_string()),
        _ => "<unknown>".to_string(),
    }
}

#[derive(Default)]
struct FunctionScoreSummary {
    cost: f64,
    functions_scored: usize,
}

#[derive(Default)]
struct FileScoreSummary {
    cost: f64,
    files_scored: usize,
}

#[derive(Default)]
struct ModuleScoreSummary {
    cost: f64,
    modules_scored: usize,
}

#[derive(Default)]
struct AbstractionScoreSummary {
    cost: f64,
    symbols_scored: usize,
}

#[derive(Default)]
struct StaleScoreSummary {
    cost: f64,
    candidate_count: usize,
}

fn score_functions(
    functions: &[FunctionScoreInput],
    scopes: &mut BTreeMap<String, ScopeAccumulator>,
    contributors: &mut Vec<Contributor>,
) -> FunctionScoreSummary {
    let mut summary = FunctionScoreSummary {
        functions_scored: functions.len(),
        ..Default::default()
    };

    for function in functions {
        let branch_cost = 2.0 * excess(function.cyclomatic, 5);
        let nesting_cost = 3.0 * excess(function.nesting_depth, 3);
        let span_cost = 0.2 * excess(function.line_count, 50);
        let body_cost = 0.5 * excess(function.body_stmt_count, 20);
        let signature_cost = excess(function.signature_score, 6);
        let cost = branch_cost + nesting_cost + span_cost + body_cost + signature_cost;

        summary.cost += cost;
        add_scope_cost(scopes, &function.scope, cost);

        if cost > 0.0 {
            let mut reasons = BTreeMap::new();
            reasons.insert("cyclomatic".to_string(), json!(function.cyclomatic));
            reasons.insert("nesting_depth".to_string(), json!(function.nesting_depth));
            reasons.insert("line_count".to_string(), json!(function.line_count));
            reasons.insert(
                "body_stmt_count".to_string(),
                json!(function.body_stmt_count),
            );
            reasons.insert(
                "signature_score".to_string(),
                json!(function.signature_score),
            );

            let mut component_costs = BTreeMap::new();
            component_costs.insert("branching".to_string(), branch_cost);
            component_costs.insert("nesting".to_string(), nesting_cost);
            component_costs.insert("span".to_string(), span_cost);
            component_costs.insert("body_statements".to_string(), body_cost);
            component_costs.insert("signature".to_string(), signature_cost);

            contributors.push(Contributor {
                kind: "function".to_string(),
                scope: function.scope.clone(),
                file: Some(function.file.clone()),
                module: None,
                function: Some(function.function.clone()),
                cost,
                reasons,
                component_costs,
            });
        }
    }

    summary
}

fn score_files(
    symbols: &ast_parser::ProjectSymbols,
    loc_by_file: &HashMap<PathBuf, loc::FileLocStats>,
    project_path: &Path,
    file_scopes: &HashMap<PathBuf, String>,
    scopes: &mut BTreeMap<String, ScopeAccumulator>,
    contributors: &mut Vec<Contributor>,
) -> FileScoreSummary {
    let mut summary = FileScoreSummary {
        files_scored: symbols.files.len(),
        ..Default::default()
    };

    for file in &symbols.files {
        let code_lines = loc_by_file
            .get(&file.path)
            .map(|stats| stats.code_lines)
            .unwrap_or_default();
        let total_symbols = total_symbols(file);
        let function_count = file.functions.len();
        let line_cost = 0.04 * excess(code_lines, 400);
        let symbol_cost = 0.25 * excess(total_symbols, 35);
        let function_count_cost = excess(function_count, 25);
        let cost = line_cost + symbol_cost + function_count_cost;
        let scope = file_scopes
            .get(&file.path)
            .cloned()
            .unwrap_or_else(|| file_scope(&file.path, project_path));

        summary.cost += cost;
        add_scope_cost(scopes, &scope, cost);

        if cost > 0.0 {
            let mut reasons = BTreeMap::new();
            reasons.insert("code_lines".to_string(), json!(code_lines));
            reasons.insert("total_symbols".to_string(), json!(total_symbols));
            reasons.insert("function_count".to_string(), json!(function_count));

            let mut component_costs = BTreeMap::new();
            component_costs.insert("code_lines".to_string(), line_cost);
            component_costs.insert("symbols".to_string(), symbol_cost);
            component_costs.insert("functions".to_string(), function_count_cost);

            contributors.push(Contributor {
                kind: "file".to_string(),
                scope,
                file: Some(relative_file(&file.path, project_path)),
                module: None,
                function: None,
                cost,
                reasons,
                component_costs,
            });
        }
    }

    summary
}

fn score_abstraction_surface(
    symbols: &ast_parser::ProjectSymbols,
    project_path: &Path,
    file_scopes: &HashMap<PathBuf, String>,
    scopes: &mut BTreeMap<String, ScopeAccumulator>,
    contributors: &mut Vec<Contributor>,
) -> AbstractionScoreSummary {
    let mut summary = AbstractionScoreSummary::default();

    for file in &symbols.files {
        let structs = file.structs.len();
        let enums = file.enums.len();
        let traits = file.traits.len();
        let trait_impls = file
            .impls
            .iter()
            .filter(|imp| imp.trait_name.is_some())
            .count();
        let consts_statics = file.consts.len() + file.statics.len();
        summary.symbols_scored += total_symbols(file);

        let struct_cost = 0.5 * excess(structs, 12);
        let enum_cost = 0.5 * excess(enums, 8);
        let trait_cost = excess(traits, 4);
        let trait_impl_cost = 0.5 * excess(trait_impls, 8);
        let const_static_cost = 0.2 * excess(consts_statics, 10);
        let cost = struct_cost + enum_cost + trait_cost + trait_impl_cost + const_static_cost;
        let scope = file_scopes
            .get(&file.path)
            .cloned()
            .unwrap_or_else(|| file_scope(&file.path, project_path));

        summary.cost += cost;
        add_scope_cost(scopes, &scope, cost);

        if cost > 0.0 {
            let mut reasons = BTreeMap::new();
            reasons.insert("structs".to_string(), json!(structs));
            reasons.insert("enums".to_string(), json!(enums));
            reasons.insert("traits".to_string(), json!(traits));
            reasons.insert("trait_impls".to_string(), json!(trait_impls));
            reasons.insert("consts_and_statics".to_string(), json!(consts_statics));

            let mut component_costs = BTreeMap::new();
            component_costs.insert("structs".to_string(), struct_cost);
            component_costs.insert("enums".to_string(), enum_cost);
            component_costs.insert("traits".to_string(), trait_cost);
            component_costs.insert("trait_impls".to_string(), trait_impl_cost);
            component_costs.insert("consts_and_statics".to_string(), const_static_cost);

            contributors.push(Contributor {
                kind: "abstraction_surface".to_string(),
                scope,
                file: Some(relative_file(&file.path, project_path)),
                module: None,
                function: None,
                cost,
                reasons,
                component_costs,
            });
        }
    }

    summary
}

fn score_module_coupling(
    files: &[PathBuf],
    project_path: &Path,
    file_scopes: &HashMap<PathBuf, String>,
    scopes: &mut BTreeMap<String, ScopeAccumulator>,
    contributors: &mut Vec<Contributor>,
) -> ModuleScoreSummary {
    let analysis = deps::analyze_deps(files, project_path);
    let cohesion = deps::compute_cohesion(files, project_path);
    let module_scopes = module_scopes(files, project_path, file_scopes);
    let mut summary = ModuleScoreSummary {
        modules_scored: analysis.modules.len(),
        ..Default::default()
    };

    for module in &analysis.modules {
        let in_degree = analysis.in_degree.get(module).copied().unwrap_or(0);
        let out_degree = analysis.out_degree.get(module).copied().unwrap_or(0);
        let out_cost = 2.0 * excess(out_degree, 5);
        let in_cost = excess(in_degree, 10);
        let cost = out_cost + in_cost;
        let scope = module_scopes
            .get(module)
            .cloned()
            .unwrap_or_else(|| "production".to_string());

        summary.cost += cost;
        add_scope_cost(scopes, &scope, cost);

        if cost > 0.0 {
            let mut reasons = BTreeMap::new();
            reasons.insert("in_degree".to_string(), json!(in_degree));
            reasons.insert("out_degree".to_string(), json!(out_degree));

            let mut component_costs = BTreeMap::new();
            component_costs.insert("fan_out".to_string(), out_cost);
            component_costs.insert("fan_in".to_string(), in_cost);

            contributors.push(Contributor {
                kind: "module".to_string(),
                scope,
                file: None,
                module: Some(module.clone()),
                function: None,
                cost,
                reasons,
                component_costs,
            });
        }
    }

    for (module_a, module_b, strength) in &analysis.coupling_pairs {
        if *strength < 2 {
            continue;
        }
        let cost = 20.0 + 5.0 * *strength as f64;
        let scope = pair_scope(module_a, module_b, &module_scopes);
        summary.cost += cost;
        add_scope_cost(scopes, &scope, cost);

        let mut reasons = BTreeMap::new();
        reasons.insert("strength".to_string(), json!(strength));
        reasons.insert("direction".to_string(), json!("bidirectional"));

        let mut component_costs = BTreeMap::new();
        component_costs.insert("bidirectional_pair".to_string(), cost);

        contributors.push(Contributor {
            kind: "module".to_string(),
            scope,
            file: None,
            module: Some(format!("{module_a} <-> {module_b}")),
            function: None,
            cost,
            reasons,
            component_costs,
        });
    }

    for row in &cohesion {
        if row.function_count < 4 {
            continue;
        }
        let cohesion_gap = (0.55 - row.combined).max(0.0);
        let cost = 2.0 * row.function_count as f64 * cohesion_gap;
        let scope = module_scopes
            .get(&row.module)
            .cloned()
            .unwrap_or_else(|| "production".to_string());
        summary.cost += cost;
        add_scope_cost(scopes, &scope, cost);

        if cost > 0.0 {
            let mut reasons = BTreeMap::new();
            reasons.insert("combined_cohesion".to_string(), json!(row.combined));
            reasons.insert("function_count".to_string(), json!(row.function_count));

            let mut component_costs = BTreeMap::new();
            component_costs.insert("low_cohesion".to_string(), cost);

            contributors.push(Contributor {
                kind: "module".to_string(),
                scope,
                file: None,
                module: Some(row.module.clone()),
                function: None,
                cost,
                reasons,
                component_costs,
            });
        }
    }

    summary
}

fn score_stale_surface(
    graph: &flow::CallGraph,
    functions: &[FunctionScoreInput],
    scopes: &mut BTreeMap<String, ScopeAccumulator>,
    contributors: &mut Vec<Contributor>,
) -> StaleScoreSummary {
    let report = dead_code::analyze_graph(graph);
    let function_scopes: HashMap<&str, &str> = functions
        .iter()
        .map(|function| (function.graph_name.as_str(), function.scope.as_str()))
        .collect();
    let function_files: HashMap<&str, &str> = functions
        .iter()
        .map(|function| (function.graph_name.as_str(), function.file.as_str()))
        .collect();
    let summary = StaleScoreSummary {
        candidate_count: report.candidate_count,
        cost: 0.5 * report.candidate_count as f64,
    };

    for candidate in &report.candidates {
        let cost = 0.5;
        let scope = function_scopes
            .get(candidate.function.as_str())
            .map(|scope| (*scope).to_string())
            .unwrap_or_else(|| graph_function_scope(graph, &candidate.function));
        add_scope_cost(scopes, &scope, cost);

        let mut reasons = BTreeMap::new();
        reasons.insert("reason".to_string(), json!(candidate.reason));

        let mut component_costs = BTreeMap::new();
        component_costs.insert("stale_candidate".to_string(), cost);

        contributors.push(Contributor {
            kind: "stale_surface".to_string(),
            scope,
            file: function_files
                .get(candidate.function.as_str())
                .map(|file| (*file).to_string()),
            module: None,
            function: Some(candidate.function.clone()),
            cost,
            reasons,
            component_costs,
        });
    }

    summary
}

fn collect_function_inputs(
    symbols: &ast_parser::ProjectSymbols,
    graph: &flow::CallGraph,
    project_path: &Path,
    file_scopes: &HashMap<PathBuf, String>,
) -> Vec<FunctionScoreInput> {
    let mut inputs = Vec::new();

    for file in &symbols.files {
        let file_module = render::module_name_from_path(&file.path, project_path);
        let rel_file = relative_file(&file.path, project_path);
        let base_scope = file_scopes
            .get(&file.path)
            .cloned()
            .unwrap_or_else(|| file_scope(&file.path, project_path));

        for function in &file.functions {
            let graph_name = graph_name_for_function(&file_module, &function.name);
            let scope = function_scope(graph, &graph_name, &base_scope, function.is_test_support);
            let line_count = function_line_count(function);
            let signature_score = signature_score(function);
            inputs.push(FunctionScoreInput {
                file: rel_file.clone(),
                function: function.name.clone(),
                graph_name,
                scope,
                cyclomatic: function.branch_points + 1,
                nesting_depth: function.nesting_depth,
                line_count,
                body_stmt_count: function.body_stmt_count,
                signature_score,
            });
        }
    }

    inputs
}

fn collect_loc_by_file(files: &[PathBuf]) -> HashMap<PathBuf, loc::FileLocStats> {
    files
        .iter()
        .filter_map(|file| loc::count_file_lines(file).map(|stats| (file.clone(), stats)))
        .collect()
}

fn collect_file_scopes(files: &[PathBuf], project_path: &Path) -> HashMap<PathBuf, String> {
    files
        .iter()
        .map(|file| (file.clone(), file_scope(file, project_path)))
        .collect()
}

fn initial_scope_totals(
    files: &[PathBuf],
    symbols: &ast_parser::ProjectSymbols,
    project_path: &Path,
    file_scopes: &HashMap<PathBuf, String>,
) -> BTreeMap<String, ScopeAccumulator> {
    let mut scopes = BTreeMap::new();
    for standard in ["production", "tests", "benches"] {
        scopes.insert(standard.to_string(), ScopeAccumulator::default());
    }

    for file in files {
        let scope = file_scopes
            .get(file)
            .cloned()
            .unwrap_or_else(|| file_scope(file, project_path));
        scopes.entry(scope).or_default().files_scored += 1;
    }

    for file in &symbols.files {
        let scope = file_scopes
            .get(&file.path)
            .cloned()
            .unwrap_or_else(|| file_scope(&file.path, project_path));
        scopes.entry(scope).or_default().functions_scored += file.functions.len();
    }

    scopes
}

fn finalize_scope_totals(
    scopes: BTreeMap<String, ScopeAccumulator>,
) -> BTreeMap<String, ScopeSummary> {
    scopes
        .into_iter()
        .map(|(scope, summary)| {
            (
                scope,
                ScopeSummary {
                    cost: round1(summary.cost),
                    functions_scored: summary.functions_scored,
                    files_scored: summary.files_scored,
                },
            )
        })
        .collect()
}

fn add_scope_cost(scopes: &mut BTreeMap<String, ScopeAccumulator>, scope: &str, cost: f64) {
    scopes.entry(scope.to_string()).or_default().cost += cost;
}

fn module_scopes(
    files: &[PathBuf],
    project_path: &Path,
    file_scopes: &HashMap<PathBuf, String>,
) -> HashMap<String, String> {
    files
        .iter()
        .map(|file| {
            let module = render::module_name_from_path(file, project_path);
            let scope = file_scopes
                .get(file)
                .cloned()
                .unwrap_or_else(|| file_scope(file, project_path));
            (module, scope)
        })
        .collect()
}

fn pair_scope(module_a: &str, module_b: &str, module_scopes: &HashMap<String, String>) -> String {
    let scope_a = module_scopes
        .get(module_a)
        .cloned()
        .unwrap_or_else(|| "production".to_string());
    let scope_b = module_scopes
        .get(module_b)
        .cloned()
        .unwrap_or_else(|| "production".to_string());
    if scope_a == scope_b {
        scope_a
    } else {
        "mixed".to_string()
    }
}

fn function_scope(
    graph: &flow::CallGraph,
    graph_name: &str,
    file_scope: &str,
    is_test_support: bool,
) -> String {
    if file_scope == "benches" || file_scope == "tests" {
        return file_scope.to_string();
    }
    if matches!(
        graph.entry_point_kinds.get(graph_name),
        Some(flow::EntryPointKind::Bench)
    ) {
        return "benches".to_string();
    }
    if matches!(
        graph.entry_point_kinds.get(graph_name),
        Some(flow::EntryPointKind::Test)
    ) {
        return "tests".to_string();
    }
    if let Some(node) = graph.function_nodes.get(graph_name) {
        if node.is_bench_entry {
            return "benches".to_string();
        }
        if node.is_test_entry || node.is_test_support {
            return "tests".to_string();
        }
    }
    if is_test_support {
        return "tests".to_string();
    }
    file_scope.to_string()
}

fn graph_function_scope(graph: &flow::CallGraph, graph_name: &str) -> String {
    if matches!(
        graph.entry_point_kinds.get(graph_name),
        Some(flow::EntryPointKind::Bench)
    ) {
        return "benches".to_string();
    }
    if matches!(
        graph.entry_point_kinds.get(graph_name),
        Some(flow::EntryPointKind::Test)
    ) {
        return "tests".to_string();
    }
    if let Some(node) = graph.function_nodes.get(graph_name) {
        if node.is_bench_entry {
            return "benches".to_string();
        }
        if node.is_test_entry || node.is_test_support {
            return "tests".to_string();
        }
    }
    "production".to_string()
}

fn file_scope(file: &Path, project_path: &Path) -> String {
    let rel = file.strip_prefix(project_path).unwrap_or(file);
    let first = rel
        .components()
        .next()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .unwrap_or_else(|| "src".to_string());
    match first.as_str() {
        "tests" => "tests".to_string(),
        "benches" => "benches".to_string(),
        "examples" => "examples".to_string(),
        _ => "production".to_string(),
    }
}

fn graph_name_for_function(file_module: &str, function_name: &str) -> String {
    if file_module == "main" || file_module == "lib" {
        function_name.to_string()
    } else {
        format!("{file_module}::{function_name}")
    }
}

fn total_symbols(file: &ast_parser::FileSymbols) -> usize {
    file.functions.len()
        + file.structs.len()
        + file.enums.len()
        + file.traits.len()
        + file.impls.len()
        + file.consts.len()
        + file.statics.len()
}

fn function_line_count(function: &ast_parser::FunctionInfo) -> usize {
    if function.line_end >= function.line_start {
        function.line_end - function.line_start + 1
    } else {
        function.body_stmt_count.max(1)
    }
}

fn signature_score(function: &ast_parser::FunctionInfo) -> usize {
    function.param_count
        + function.return_type_complexity
        + function.generic_param_count
        + function.trait_bound_count
        + function.where_predicate_count
}

fn excess(value: usize, threshold: usize) -> f64 {
    value.saturating_sub(threshold) as f64
}

fn relative_file(path: &Path, project_path: &Path) -> String {
    path.strip_prefix(project_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}
