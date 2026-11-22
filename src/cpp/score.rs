use std::borrow::Cow;
use std::cmp::Reverse;
use std::path::Path;

use serde::Serialize;

use crate::discovery::cpp::{display_path, Selection};

use super::includes::{FileCoupling, Graph};
use super::report::{line_counts, print_json, print_occurrence, print_summary, Occurrence, Rows};
use super::{Access, Analysis, Body, File, Loc, Signature, Span, Status, Symbol};

const VERSION: &str = "cpp_code_complexity_cost_v1";

fn excess_square(value: usize, threshold: usize) -> usize {
    value.saturating_sub(threshold).pow(2)
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Scope {
    Production,
    Tests,
    Benchmarks,
}

impl Scope {
    fn of(path: &Path) -> Self {
        if let Some(dir) = path.parent() {
            for part in dir.iter().rev() {
                match part.to_str() {
                    Some("test" | "tests") => return Self::Tests,
                    Some("bench" | "benches" | "benchmark" | "benchmarks") => return Self::Benchmarks,
                    _ => {}
                }
            }
        }
        Self::Production
    }

    fn label(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Tests => "tests",
            Self::Benchmarks => "benchmarks",
        }
    }
}

#[derive(Default, Serialize)]
struct Inputs {
    files: usize,
    loc: Loc,
    bodies: usize,
    decisions: usize,
    nesting_squared_sum: usize,
    statements: usize,
    parameters: usize,
    parameter_excess_squared_sum: usize,
    template_parameters: usize,
    constraints: usize,
    type_complexity: usize,
    named_surface: usize,
    eligible_header_occurrences: usize,
    internal_edges: usize,
    cyclic_edges: usize,
}

impl Inputs {
    fn add(&mut self, other: &Self) {
        self.files += other.files;
        self.loc.add(&other.loc);
        self.bodies += other.bodies;
        self.decisions += other.decisions;
        self.nesting_squared_sum += other.nesting_squared_sum;
        self.statements += other.statements;
        self.parameters += other.parameters;
        self.parameter_excess_squared_sum += other.parameter_excess_squared_sum;
        self.template_parameters += other.template_parameters;
        self.constraints += other.constraints;
        self.type_complexity += other.type_complexity;
        self.named_surface += other.named_surface;
        self.eligible_header_occurrences += other.eligible_header_occurrences;
        self.internal_edges += other.internal_edges;
        self.cyclic_edges += other.cyclic_edges;
    }

    fn record_body(&mut self, body: &Body, signature: &Signature) -> usize {
        let metrics = &body.metrics;
        let nesting_squared = metrics.max_nesting.pow(2);
        let parameter_excess_squared = excess_square(signature.parameters, 3);
        self.bodies += 1;
        self.decisions += metrics.decisions;
        self.nesting_squared_sum += nesting_squared;
        self.statements += metrics.statements;
        self.parameters += signature.parameters;
        self.parameter_excess_squared_sum += parameter_excess_squared;
        self.template_parameters += signature.template_parameters;
        self.constraints += signature.constraints;
        self.type_complexity += signature.type_complexity;
        1 + 2 * metrics.decisions + nesting_squared + metrics.statements + parameter_excess_squared
            + signature.template_parameters + 2 * signature.constraints + signature.type_complexity
    }

    fn print(&self) {
        println!("  {} files; {}", self.files, line_counts(self.loc));
        println!("  Bodies (F): {}, decisions (D): {}, sum of N²: {}, statements (S): {}, parameters (P): {}, sum of excess(P,3)²: {}, template parameters (T): {}, constraints (R): {}, type complexity (Y): {}",
            self.bodies, self.decisions, self.nesting_squared_sum, self.statements, self.parameters,
            self.parameter_excess_squared_sum, self.template_parameters, self.constraints, self.type_complexity);
        println!("  Named surface (A): {}, eligible header occurrences: {}, unique outgoing internal edges: {}, of which cyclic: {}",
            self.named_surface, self.eligible_header_occurrences, self.internal_edges, self.cyclic_edges);
    }
}

#[derive(Default, Serialize)]
struct Components {
    function_complexity_cost: usize,
    file_concentration_cost: usize,
    declaration_surface_cost: usize,
    include_coupling_cost: usize,
}

impl Components {
    fn sum(&self) -> usize {
        self.function_complexity_cost + self.file_concentration_cost
            + self.declaration_surface_cost + self.include_coupling_cost
    }
}

#[derive(Default, Serialize)]
struct Totals {
    code_complexity_cost: usize,
    components: Components,
    inputs: Inputs,
}

impl Totals {
    fn add(&mut self, other: &Self) {
        self.code_complexity_cost += other.code_complexity_cost;
        self.components.function_complexity_cost += other.components.function_complexity_cost;
        self.components.file_concentration_cost += other.components.file_concentration_cost;
        self.components.declaration_surface_cost += other.components.declaration_surface_cost;
        self.components.include_coupling_cost += other.components.include_coupling_cost;
        self.inputs.add(&other.inputs);
    }

    fn print(&self) {
        let c = &self.components;
        println!("  Code complexity cost: {} (function complexity: {}, file concentration: {}, declaration surface: {}, include coupling: {})",
            self.code_complexity_cost, c.function_complexity_cost, c.file_concentration_cost,
            c.declaration_surface_cost, c.include_coupling_cost);
        self.inputs.print();
    }
}

#[derive(Default, Serialize)]
struct Scopes {
    production: Totals,
    tests: Totals,
    benchmarks: Totals,
}

impl Scopes {
    fn get_mut(&mut self, scope: Scope) -> &mut Totals {
        match scope {
            Scope::Production => &mut self.production,
            Scope::Tests => &mut self.tests,
            Scope::Benchmarks => &mut self.benchmarks,
        }
    }

    fn print(&self) {
        for (scope, totals) in [("production", &self.production), ("tests", &self.tests), ("benchmarks", &self.benchmarks)] {
            println!("Scope: {scope} (weight 1)");
            totals.print();
        }
    }
}

#[derive(Serialize)]
struct FileScore<'a> {
    #[serde(skip)]
    path: &'a Path,
    header: bool,
    scope: Scope,
    coupling: FileCoupling,
    #[serde(flatten)]
    totals: Totals,
}

#[derive(Serialize)]
struct FileRow<'a> {
    path: Cow<'a, str>,
    #[serde(flatten)]
    file: &'a FileScore<'a>,
}

impl FileScore<'_> {
    fn row(&self) -> FileRow<'_> {
        FileRow { path: display_path(self.path), file: self }
    }
}

struct RankedSymbol<'a> {
    file: &'a File<'a>,
    symbol: &'a Symbol,
    cost: usize,
    scope: Scope,
}

impl RankedSymbol<'_> {
    fn order(&self) -> (Reverse<usize>, &Path, usize, usize, &str) {
        let span = self.symbol.span;
        (Reverse(self.cost), self.file.path, span.byte_start, span.byte_end, &self.symbol.name)
    }

    fn occurrence(&self) -> Occurrence<'_> {
        Occurrence::new(self.file, self.symbol)
    }
}

#[derive(Serialize)]
struct FunctionRow<'a> {
    #[serde(flatten)]
    occurrence: Occurrence<'a>,
    scope: Scope,
    cost: usize,
    body_span: Span,
    #[serde(flatten)]
    body_metrics: &'a super::metrics::BodyMetrics,
    parameters: usize,
    constraints: usize,
    type_complexity: usize,
    variadic: bool,
    parameter_packs: usize,
}

#[derive(Serialize)]
struct SurfaceRow<'a> {
    #[serde(flatten)]
    occurrence: Occurrence<'a>,
    scope: Scope,
    eligible_header_occurrence: bool,
    cost: usize,
}

#[derive(Serialize)]
struct CouplingRow<'a> {
    path: Cow<'a, str>,
    scope: Scope,
    #[serde(flatten)]
    coupling: FileCoupling,
    cost: usize,
}

#[derive(Serialize)]
struct ConcentrationRow<'a> {
    path: Cow<'a, str>,
    scope: Scope,
    bodies: usize,
    statements: usize,
    named_surface: usize,
    cost: usize,
}

#[derive(Serialize)]
struct Contributors<F, C, S, I> {
    function_complexity_cost: F,
    file_concentration_cost: C,
    declaration_surface_cost: S,
    include_coupling_cost: I,
}

#[derive(Serialize)]
struct Version {
    score_version: &'static str,
}

#[derive(Serialize)]
struct ScoreReport<'a, F, R> {
    score_version: &'static str,
    loc_mode: &'static str,
    #[serde(flatten)]
    totals: &'a Totals,
    scope_weight: usize,
    scopes: &'a Scopes,
    files: F,
    includes: &'a Graph,
    contributors: R,
}

fn rank_files<'a>(files: &'a [FileScore<'a>], cost: fn(&Components) -> usize) -> Vec<&'a FileScore<'a>> {
    let mut ranked: Vec<_> = files.iter().filter(|file| cost(&file.totals.components) != 0).collect();
    ranked.sort_unstable_by_key(|file| (Reverse(cost(&file.totals.components)), file.path));
    ranked
}

fn rank_symbols(rows: &mut [RankedSymbol<'_>]) {
    rows.sort_unstable_by(|a, b| a.order().cmp(&b.order()));
}

pub(super) fn run(selection: &Selection, mut analysis: Analysis<'_>, top: Option<usize>, json: bool, verbose: bool) -> bool {
    if analysis.status == Status::Complete {
        match Graph::build(selection, &analysis.files) {
            Ok(graph) => {
                render_complete(selection, &analysis, &graph, top, json, verbose);
                return true;
            }
            Err(diagnostics) => {
                analysis.diagnostics.extend(diagnostics);
                analysis.finish(selection.files.len());
            }
        }
    }
    if json {
        print_json(selection, &analysis, Version { score_version: VERSION });
    } else {
        print_summary(selection, &analysis, "scorecard");
        println!("Score version: {VERSION}; no comparable score.");
        for path in &selection.files {
            println!("Selected file: {}", display_path(path));
        }
    }
    analysis.status == Status::Empty && selection.is_changed()
}

fn render_complete(selection: &Selection, analysis: &Analysis<'_>, graph: &Graph, top: Option<usize>, json: bool, verbose: bool) {
    let mut totals = Totals::default();
    let mut scopes = Scopes::default();
    let mut files = Vec::with_capacity(analysis.files.len());
    let mut functions = Vec::new();
    let mut surface = Vec::new();
    let limit = top.unwrap_or(usize::MAX);
    for file in &analysis.files {
        let scope = Scope::of(file.path);
        let coupling = graph.file_metrics(file.path);
        let mut inputs = Inputs {
            files: 1,
            loc: file.loc.expect("score analysis collected physical LOC"),
            internal_edges: coupling.fan_out,
            cyclic_edges: coupling.cyclic_edges,
            ..Inputs::default()
        };
        let mut components = Components::default();
        for symbol in &file.symbols {
            if let Some(body) = &symbol.body {
                let signature = symbol.signature.as_ref().expect("a callable body has a signature");
                let cost = inputs.record_body(body, signature);
                components.function_complexity_cost += cost;
                if limit != 0 {
                    functions.push(RankedSymbol { file, symbol, cost, scope });
                }
            }
            if symbol.named_surface {
                let bonus = usize::from(file.header && !matches!(symbol.access, Some(Access::Private | Access::Unknown)));
                inputs.named_surface += 1;
                inputs.eligible_header_occurrences += bonus;
                let cost = 1 + bonus;
                components.declaration_surface_cost += cost;
                if limit != 0 {
                    surface.push(RankedSymbol { file, symbol, cost, scope });
                }
            }
        }
        components.file_concentration_cost = excess_square(inputs.bodies, 12)
            + excess_square(inputs.statements, 80) + excess_square(inputs.named_surface, 24);
        components.include_coupling_cost = 2 * coupling.fan_out + excess_square(coupling.fan_out, 4)
            + 3 * coupling.cyclic_edges;
        let file_totals = Totals { code_complexity_cost: components.sum(), components, inputs };
        totals.add(&file_totals);
        scopes.get_mut(scope).add(&file_totals);
        files.push(FileScore { path: file.path, header: file.header, scope, coupling, totals: file_totals });
    }
    rank_symbols(&mut functions);
    rank_symbols(&mut surface);
    functions.truncate(limit);
    surface.truncate(limit);
    let mut concentration = Vec::new();
    let mut coupling = Vec::new();
    if limit != 0 {
        concentration = rank_files(&files, |c| c.file_concentration_cost);
        coupling = rank_files(&files, |c| c.include_coupling_cost);
        concentration.truncate(limit);
        coupling.truncate(limit);
    }
    if json {
        let contributors = Contributors {
            function_complexity_cost: Rows(functions.iter().map(|r| {
                let body = r.symbol.body.as_ref().unwrap();
                let signature = r.symbol.signature.as_ref().unwrap();
                FunctionRow {
                    occurrence: r.occurrence(), scope: r.scope, cost: r.cost, body_span: body.span,
                    body_metrics: &body.metrics, parameters: signature.parameters, constraints: signature.constraints,
                    type_complexity: signature.type_complexity, variadic: signature.variadic, parameter_packs: signature.parameter_packs,
                }
            })),
            file_concentration_cost: Rows(concentration.iter().map(|file| ConcentrationRow {
                path: display_path(file.path), scope: file.scope, bodies: file.totals.inputs.bodies,
                statements: file.totals.inputs.statements, named_surface: file.totals.inputs.named_surface,
                cost: file.totals.components.file_concentration_cost,
            })),
            declaration_surface_cost: Rows(surface.iter().map(|r| SurfaceRow {
                occurrence: r.occurrence(), scope: r.scope, eligible_header_occurrence: r.cost == 2, cost: r.cost,
            })),
            include_coupling_cost: Rows(coupling.iter().map(|file| CouplingRow {
                path: display_path(file.path), scope: file.scope, coupling: file.coupling, cost: file.totals.components.include_coupling_cost,
            })),
        };
        print_json(selection, analysis, ScoreReport {
            score_version: VERSION, loc_mode: "physical", totals: &totals, scope_weight: 1, scopes: &scopes,
            files: Rows(files.iter().map(FileScore::row)), includes: graph, contributors,
        });
        return;
    }

    print_summary(selection, analysis, "scorecard");
    println!("Score version: {VERSION}; compare complete runs with equivalent selection rules.");
    println!("The score sums four costs (physical LOC does not enter the score):");
    println!("  Function complexity per body: 1 + 2D + N² + S + excess(P,3)² + T + 2R + Y (N = max nesting)");
    println!("  File concentration per file: excess(F,12)² + excess(S,80)² + excess(A,24)²");
    println!("  Declaration surface per named occurrence: 1 + eligible header occurrence");
    println!("  Include coupling per file: 2 × fan-out + excess(fan-out,4)² + 3 × cyclic outgoing edges");
    println!("  excess(x,k) = max(0,x−k). All scopes have equal weight.");
    println!("  The header bonus excludes private and unknown-access members; it does not imply ABI visibility.");
    println!("Totals:");
    totals.print();
    scopes.print();
    println!("Selected files (path order):");
    for file in &files {
        println!("{} [{}; {}; fan-in: {}, fan-out: {}]", display_path(file.path), file.scope.label(),
            if file.header { "header" } else { "source" }, file.coupling.fan_in, file.coupling.fan_out);
        file.totals.print();
    }
    graph.print();
    if let Some(top) = top {
        println!("Ranked contributors (up to {top} per component):");
    } else {
        println!("Ranked contributors:");
    }
    println!("Function complexity:");
    for r in &functions {
        let body = r.symbol.body.as_ref().unwrap();
        let signature = r.symbol.signature.as_ref().unwrap();
        print_occurrence(r.file, r.symbol);
        println!("; cost: {}; D={}, N={}, S={}, P={}, T={}, R={}, Y={}", r.cost, body.metrics.decisions,
            body.metrics.max_nesting, body.metrics.statements, signature.parameters, signature.template_parameters,
            signature.constraints, signature.type_complexity);
        if verbose {
            let b = &body.metrics.categories;
            println!("  if: {}, for: {}, range for: {}, while: {}, do: {}, case: {}, default: {}, catch: {}, ?: {}, &&/and: {}, ||/or: {}",
                b.if_statements, b.for_loops, b.range_for_loops, b.while_loops, b.do_while_loops, b.case_labels,
                b.default_labels, b.catch_clauses, b.conditional_expressions, b.logical_and, b.logical_or);
        }
    }
    println!("File concentration:");
    for file in concentration {
        let inputs = &file.totals.inputs;
        println!("{}: cost: {}; F={}, S={}, A={}", display_path(file.path), file.totals.components.file_concentration_cost,
            inputs.bodies, inputs.statements, inputs.named_surface);
    }
    println!("Declaration surface:");
    for r in &surface {
        print_occurrence(r.file, r.symbol);
        println!("; cost: {}; eligible header occurrence: {}", r.cost, r.cost == 2);
    }
    println!("Include coupling:");
    for file in coupling {
        println!("{}: cost: {}; fan-out: {}, cyclic outgoing edges: {}", display_path(file.path),
            file.totals.components.include_coupling_cost, file.coupling.fan_out, file.coupling.cyclic_edges);
    }
}
