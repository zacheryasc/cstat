use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::io::{self, Write};

use serde::Serialize;

use super::metrics::Loc;
use super::{analyze, Access, Analysis, Diagnostic, File, Inputs, Kind, Role, Span, Status, Symbol};
use crate::discovery::cpp::{display_path, Selection};

const LIMITATIONS: &[&str] = &[
    "No build, macro expansion, template instantiation, or semantic lookup (including overload resolution).",
    "Conditional branches are analyzed where parsed, not evaluated for an active build configuration.",
    "Bare uppercase annotations after noexcept are read as opaque tokens, not expanded.",
];

#[derive(Clone, Copy)]
pub(crate) enum Report {
    Loc { top: Option<usize> },
    Symbols,
    Branching,
    Signature,
    Span,
    Scorecard { top: Option<usize> },
    DiffHunks { top: Option<usize> },
}

impl Report {
    fn heading(self) -> &'static str {
        match self {
            Self::Loc { .. } => "Lines of Code (physical source)",
            Self::Symbols => "Symbols",
            Self::Branching => "Branching",
            Self::Signature => "Signatures",
            Self::Span => "Implementation Spans",
            Self::Scorecard { .. } => "Scorecard",
            Self::DiffHunks { .. } => "Diff-hunk Scorecard",
        }
    }
}

pub(crate) fn explain_loc(json: bool) {
    const EXPLANATION: &str = "C++ LOC counts physical blank, comment-only, and code lines in directories and selected files, without formatting or static reachability.";
    if json {
        println!("{}", serde_json::json!({
            "language": "cpp",
            "loc_mode": "physical",
            "explanation": EXPLANATION,
            "line_counts": ["total_lines", "code_lines", "comment_lines", "blank_lines"],
        }));
    } else {
        println!("{EXPLANATION}");
    }
}

pub(crate) fn run(selection: &Selection, report: Report, json: bool, verbose: bool) -> bool {
    let inputs = match report {
        Report::Loc { .. } => Inputs::Loc,
        Report::Scorecard { .. } => Inputs::Scorecard,
        Report::DiffHunks { top } => Inputs::DiffHunks { collect_contributors: top != Some(0) },
        _ => Inputs::Symbols,
    };
    let analysis = analyze(selection, inputs);
    if let Report::Scorecard { top } = report {
        return super::score::run(selection, analysis, top, json, verbose);
    }
    if let Report::DiffHunks { top } = report {
        return super::hunks::run(selection, &analysis, top, json);
    }
    if !json {
        print_summary(selection, &analysis, report.heading());
    }
    match report {
        Report::Loc { top } => render_loc(selection, &analysis, top, json),
        Report::Symbols => render_symbols(selection, &analysis, json),
        Report::Branching | Report::Signature | Report::Span => {
            render_callables(selection, &analysis, report, json, verbose);
        }
        Report::Scorecard { .. } | Report::DiffHunks { .. } => unreachable!(),
    }
    analysis.status == Status::Complete
}

#[derive(Serialize)]
struct Metadata<'a> {
    language: &'static str,
    analysis_mode: &'static str,
    status: Status,
    selection: serde_json::Value,
    selected_files: Vec<Cow<'a, str>>,
    file_counts: FileCounts,
    diagnostics: &'a [Diagnostic],
    limitations: &'static [&'static str],
}

#[derive(Serialize)]
struct FileCounts {
    selected: usize,
    parsed: usize,
    failed: usize,
}

impl FileCounts {
    fn new(selection: &Selection, analysis: &Analysis<'_>) -> Self {
        Self {
            selected: selection.files.len(),
            parsed: analysis.parsed,
            failed: selection.files.len() - analysis.parsed,
        }
    }
}

#[derive(Serialize)]
struct Document<'a, T> {
    #[serde(flatten)]
    metadata: Metadata<'a>,
    #[serde(flatten)]
    report: T,
}

pub(super) fn print_json(selection: &Selection, analysis: &Analysis<'_>, report: impl Serialize) {
    let document = Document {
        metadata: Metadata {
            language: "cpp",
            analysis_mode: "source_syntax",
            status: analysis.status,
            selection: selection.rules_json(),
            selected_files: selection.files.iter().map(|file| display_path(file)).collect(),
            file_counts: FileCounts::new(selection, analysis),
            diagnostics: &analysis.diagnostics,
            limitations: LIMITATIONS,
        },
        report,
    };
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, &document).unwrap();
    writeln!(output).unwrap();
}

pub(super) struct Rows<I>(pub(super) I);

impl<I> Serialize for Rows<I>
where
    I: Iterator + Clone,
    I::Item: Serialize,
{
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.clone())
    }
}

pub(super) fn print_summary(selection: &Selection, analysis: &Analysis<'_>, heading: &str) {
    println!("C++ {heading} ({})", analysis.status.label());
    println!("Analysis mode: source_syntax");
    selection.print_rules();
    let counts = FileCounts::new(selection, analysis);
    println!("Files: {} selected, {} parsed cleanly, {} failed", counts.selected, counts.parsed, counts.failed);
    for limitation in LIMITATIONS {
        println!("{limitation}");
    }
    if analysis.status == Status::Empty {
        println!("No C++ inputs matched the selection.");
    } else if analysis.status == Status::Incomplete {
        println!("Observations below are partial.");
    }
    for diagnostic in &analysis.diagnostics {
        let file = &diagnostic.file;
        let kind = diagnostic.kind;
        let message = &diagnostic.message;
        if let (Some(line), Some(column)) = (diagnostic.line, diagnostic.column) {
            println!("{file}:{line}:{column}: {kind}: {message}");
        } else {
            println!("{file}: {kind}: {message}");
        }
    }
}

#[derive(Serialize)]
struct LocRow<'a> {
    path: Cow<'a, str>,
    #[serde(flatten)]
    loc: Loc,
}

#[derive(Serialize)]
struct LocReport<T> {
    loc_mode: &'static str,
    totals: Loc,
    files: T,
}

fn render_loc(selection: &Selection, analysis: &Analysis<'_>, top: Option<usize>, json: bool) {
    let mut totals = Loc::default();
    let mut files: Vec<_> = analysis
        .files
        .iter()
        .map(|file| {
            let loc = file.loc.as_ref().expect("LOC analysis collected line counts");
            totals.add(loc);
            (file.path, loc)
        })
        .collect();
    files.sort_unstable_by_key(|(path, loc)| (Reverse(loc.code_lines), *path));
    if let Some(limit) = top.filter(|_| !selection.is_file()) {
        files.truncate(limit);
    }
    if json {
        print_json(
            selection,
            analysis,
            LocReport {
                loc_mode: "physical",
                totals,
                files: Rows(files.iter().map(|&(path, loc)| LocRow {
                    path: display_path(path),
                    loc: *loc,
                })),
            },
        );
    } else {
        println!("Total: {}", line_counts(totals));
        for (path, loc) in files {
            println!("{}: {}", display_path(path), line_counts(*loc));
        }
    }
}

pub(super) fn line_counts(loc: Loc) -> impl std::fmt::Display {
    struct Counts(Loc);
    impl std::fmt::Display for Counts {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let Loc { total_lines, code_lines, comment_lines, blank_lines } = self.0;
            write!(f, "{total_lines} physical lines, {code_lines} code, {comment_lines} comment-only, {blank_lines} blank")
        }
    }
    Counts(loc)
}

#[derive(Serialize)]
pub(super) struct Occurrence<'a> {
    file: Cow<'a, str>,
    name: &'a str,
    kind: Kind,
    role: Role,
    span: Span,
    header: bool,
    access: Option<Access>,
    template_heads: usize,
    template_parameters: usize,
}

impl<'a> Occurrence<'a> {
    pub(super) fn new(file: &'a File<'_>, symbol: &'a Symbol) -> Self {
        Self {
            file: display_path(file.path),
            name: &symbol.name,
            kind: symbol.kind,
            role: symbol.role,
            span: symbol.span,
            header: file.header,
            access: symbol.access,
            template_heads: symbol.template_heads,
            template_parameters: symbol.template_parameters,
        }
    }
}

#[derive(Serialize)]
struct SymbolRow<'a> {
    #[serde(flatten)]
    occurrence: Occurrence<'a>,
    named_surface: bool,
}

#[derive(Serialize)]
struct SymbolsReport<T> {
    counts_by_kind: BTreeMap<Kind, usize>,
    symbols: T,
}

fn render_symbols(selection: &Selection, analysis: &Analysis<'_>, json: bool) {
    let mut counts = BTreeMap::new();
    let symbols = analysis.files.iter().flat_map(|file| file.symbols.iter().map(move |symbol| (file, symbol)));
    for (_, symbol) in symbols.clone() {
        *counts.entry(symbol.kind).or_insert(0) += 1;
    }
    if json {
        print_json(selection, analysis, SymbolsReport {
            counts_by_kind: counts,
            symbols: Rows(symbols.map(|(file, symbol)| SymbolRow {
                occurrence: Occurrence::new(file, symbol),
                named_surface: symbol.named_surface,
            })),
        });
    } else {
        for (kind, count) in counts {
            println!("{}: {count}", kind.label());
        }
        for (file, symbol) in symbols {
            print_occurrence(file, symbol);
            println!("; named surface: {}", symbol.named_surface);
        }
    }
}

#[derive(Serialize)]
struct CallableRows<T> {
    functions: T,
}

#[derive(Serialize)]
struct CallableRow<'a, T> {
    #[serde(flatten)]
    occurrence: Occurrence<'a>,
    #[serde(flatten)]
    details: T,
}

#[derive(Serialize)]
struct SignatureDetails {
    parameters: usize,
    constraints: usize,
    type_complexity: usize,
    variadic: bool,
    parameter_packs: usize,
}

#[derive(Serialize)]
struct SpanDetails {
    body_span: Span,
    body_lines: usize,
    statements: usize,
}

fn render_callables(selection: &Selection, analysis: &Analysis<'_>, report: Report, json: bool, verbose: bool) {
    let mut rows = Vec::new();
    for file in &analysis.files {
        for symbol in &file.symbols {
            let rank = match report {
                Report::Branching => symbol.body.as_ref().map(|body| body.metrics.decisions),
                Report::Signature => symbol.signature.as_ref().map(|s| s.parameters + s.template_parameters + s.constraints + s.type_complexity),
                Report::Span => symbol.body.as_ref().map(|body| body.span.lines()),
                _ => unreachable!(),
            };
            if let Some(rank) = rank {
                rows.push((file, symbol, rank));
            }
        }
    }
    rows.sort_unstable_by_key(|&(file, symbol, rank)| {
        (Reverse(rank), file.path, symbol.span.byte_start, symbol.span.byte_end, &symbol.name)
    });
    let rows = rows.iter().map(|&(file, symbol, _)| (file, symbol));
    if json {
        match report {
            Report::Branching => print_callables(selection, analysis, rows, |symbol| &symbol.body.as_ref().unwrap().metrics),
            Report::Signature => print_callables(selection, analysis, rows, |symbol| {
                let signature = symbol.signature.as_ref().unwrap();
                SignatureDetails {
                    parameters: signature.parameters,
                    constraints: signature.constraints,
                    type_complexity: signature.type_complexity,
                    variadic: signature.variadic,
                    parameter_packs: signature.parameter_packs,
                }
            }),
            Report::Span => print_callables(selection, analysis, rows, |symbol| {
                let body = symbol.body.as_ref().unwrap();
                SpanDetails { body_span: body.span, body_lines: body.span.lines(), statements: body.metrics.statements }
            }),
            _ => unreachable!(),
        }
        return;
    }
    if verbose {
        println!("{}", match report {
            Report::Branching => "Ranked by decisions; path count is one plus decisions. Nested callable bodies are counted separately.",
            Report::Signature => "Ranked by parameters + template parameters + constraints + type complexity.",
            Report::Span => "Ranked by physical body lines; statements exclude nested callable bodies.",
            _ => unreachable!(),
        });
    }
    for (file, symbol) in rows {
        print_occurrence(file, symbol);
        match report {
            Report::Branching => {
                let metrics = &symbol.body.as_ref().unwrap().metrics;
                println!("; decisions: {}, paths: {}, max nesting: {}, statements: {}", metrics.decisions, metrics.path_count, metrics.max_nesting, metrics.statements);
                let b = &metrics.categories;
                println!("  if: {}, for: {}, range for: {}, while: {}, do: {}, case: {}, default: {}, catch: {}, ?: {}, &&/and: {}, ||/or: {}", b.if_statements, b.for_loops, b.range_for_loops, b.while_loops, b.do_while_loops, b.case_labels, b.default_labels, b.catch_clauses, b.conditional_expressions, b.logical_and, b.logical_or);
            }
            Report::Signature => {
                let s = symbol.signature.as_ref().unwrap();
                println!("; parameters: {}, constraints: {}, type complexity: {}, C variadic: {}, parameter packs: {}", s.parameters, s.constraints, s.type_complexity, s.variadic, s.parameter_packs);
            }
            Report::Span => {
                let body = symbol.body.as_ref().unwrap();
                println!("; body lines {}–{} ({} physical lines), bytes {}..{}, {} statements", body.span.line_start, body.span.line_end, body.span.lines(), body.span.byte_start, body.span.byte_end, body.metrics.statements);
            }
            _ => unreachable!(),
        }
    }
}

fn print_callables<'a, T: Serialize>(
    selection: &Selection,
    analysis: &Analysis<'_>,
    rows: impl Iterator<Item = (&'a File<'a>, &'a Symbol)> + Clone,
    details: impl Fn(&'a Symbol) -> T + Clone,
) {
    print_json(selection, analysis, CallableRows {
        functions: Rows(rows.map(move |(file, symbol)| CallableRow {
            occurrence: Occurrence::new(file, symbol),
            details: details(symbol),
        })),
    });
}

pub(super) fn print_occurrence(file: &File<'_>, symbol: &Symbol) {
    let span = symbol.span;
    print!("{}:{}–{} {} ({} {}; bytes {}..{}", display_path(file.path), span.line_start, span.line_end, symbol.name, symbol.kind.label(), symbol.role.label(), span.byte_start, span.byte_end);
    if file.header {
        print!("; header");
    }
    if let Some(access) = symbol.access {
        print!("; {} access", match access {
            Access::Public => "public",
            Access::Protected => "protected",
            Access::Private => "private",
            Access::Unknown => "unknown",
        });
    }
    if symbol.template_heads != 0 {
        print!("; template heads: {}", symbol.template_heads);
    }
    if symbol.signature.is_some() || symbol.template_heads != 0 {
        print!("; template parameters: {}", symbol.template_parameters);
    }
    print!(")");
}
