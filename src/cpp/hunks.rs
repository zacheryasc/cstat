use std::borrow::Cow;
use std::cmp::Reverse;
use std::path::Path;

use serde::Serialize;
use tree_sitter::{Node, Point};

use crate::discovery::cpp::{display_path, AddedLines, LineRange, Selection};

use super::metrics::visit_body;
use super::report::{print_json, print_summary, Rows};
use super::signature::{layer_token, SignaturePart};
use super::{Access, Analysis, Status, Symbol};

const VERSION: &str = "cpp_diff_hunk_complexity_cost_v1";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Body,
    Decision,
    Statement,
    TemplateParameter,
    Constraint,
    TypeComplexity,
    NamedSurface,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Decision => "decision",
            Self::Statement => "statement",
            Self::TemplateParameter => "template_parameter",
            Self::Constraint => "constraint",
            Self::TypeComplexity => "type_complexity",
            Self::NamedSurface => "named_surface",
        }
    }
}

#[derive(Serialize)]
struct Contribution {
    line: usize,
    column: usize,
    kind: Kind,
    syntax: &'static str,
    cost: usize,
}

enum Ranges<'a> {
    Full(Option<LineRange>),
    Diff(&'a [LineRange]),
}

impl Ranges<'_> {
    fn as_slice(&self) -> &[LineRange] {
        match self {
            Self::Full(range) => range.as_slice(),
            Self::Diff(ranges) => ranges,
        }
    }
}

pub(super) struct FileScore<'a> {
    ranges: Ranges<'a>,
    contributors: Option<Vec<Contribution>>,
    cost: usize,
}

impl<'a> FileScore<'a> {
    pub(super) fn new(added: &'a AddedLines, total_lines: usize, collect_contributors: bool) -> Option<Self> {
        let ranges = match added {
            AddedLines::All => Ranges::Full((total_lines != 0).then_some(LineRange { start: 1, end: total_lines })),
            AddedLines::Ranges(ranges) if ranges.last().is_none_or(|range| range.end <= total_lines) => {
                Ranges::Diff(ranges)
            }
            AddedLines::Ranges(_) => return None,
        };
        Some(Self { ranges, contributors: collect_contributors.then(Vec::new), cost: 0 })
    }

    fn record(&mut self, anchor: Point, kind: Kind, syntax: &'static str, cost: usize) -> bool {
        let line = anchor.row + 1;
        let ranges = self.ranges.as_slice();
        let index = ranges.partition_point(|range| range.end < line);
        if !ranges.get(index).is_some_and(|range| range.start <= line) {
            return false;
        }
        self.cost += cost;
        if let Some(contributors) = &mut self.contributors {
            contributors.push(Contribution { line, column: anchor.column + 1, kind, syntax, cost });
        }
        true
    }

    pub(super) fn body(&mut self, owner: Node<'_>, body: Node<'_>) {
        self.record(body.start_position(), Kind::Body, body.kind(), 1);
        visit_body(owner, body, |node, decision, statement, _| {
            if let Some(decision) = decision {
                let anchor = decision.anchor(node);
                self.record(anchor.start_position(), Kind::Decision, anchor.kind(), 2);
            }
            if statement {
                self.record(node.start_position(), Kind::Statement, node.kind(), 1);
            }
        });
    }

    pub(super) fn signature(&mut self, anchor: Node<'_>, part: SignaturePart) -> usize {
        let (kind, cost) = match part {
            SignaturePart::TemplateParameter => (Kind::TemplateParameter, 1),
            SignaturePart::Constraint => (Kind::Constraint, 2),
            SignaturePart::TypeComplexity | SignaturePart::TypeLayer(_) => (Kind::TypeComplexity, 1),
        };
        let anchor = match part {
            SignaturePart::TypeLayer(token) => layer_token(anchor, token),
            _ => anchor,
        };
        usize::from(self.record(anchor.start_position(), kind, anchor.kind(), cost))
    }

    pub(super) fn finish(&mut self, symbols: &[Symbol], header: bool) {
        for symbol in symbols.iter().filter(|symbol| symbol.named_surface) {
            let bonus = header && !matches!(symbol.access, Some(Access::Private | Access::Unknown));
            self.record(symbol.name_position, Kind::NamedSurface, symbol.kind.label(), 1 + usize::from(bonus));
        }
        if let Some(contributors) = &mut self.contributors {
            contributors.sort_unstable_by_key(|c| (c.line, c.column, c.kind, c.syntax, c.cost));
        }
    }
}

#[derive(Serialize)]
struct FileRow<'a> {
    path: Cow<'a, str>,
    added_lines: &'a [LineRange],
    diff_hunk_complexity_cost: usize,
}

#[derive(Serialize)]
struct ContributorRow<'a> {
    path: Cow<'a, str>,
    #[serde(flatten)]
    contributor: &'a Contribution,
}

#[derive(Serialize)]
struct Report<F, C> {
    score_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff_hunk_complexity_cost: Option<usize>,
    files: F,
    contributors: C,
}

pub(super) fn run(selection: &Selection, analysis: &Analysis<'_>, top: Option<usize>, json: bool) -> bool {
    let complete = analysis.status != Status::Incomplete;
    let files = analysis.files.iter().filter_map(|file| Some((file.path, file.hunks.as_ref()?)));
    let cost = complete.then(|| files.clone().map(|(_, score)| score.cost).sum());
    let contributors = files.clone().flat_map(|(path, score)| {
        score.contributors.iter().flatten().map(move |c| (path, c))
    });

    if !complete || top == Some(0) {
        render(selection, analysis, cost, files, std::iter::empty(), json);
    } else if let Some(top) = top {
        let mut ranked: Vec<_> = contributors.collect();
        ranked.sort_unstable_by_key(|&(path, c)| (Reverse(c.cost), path, c.line, c.column, c.kind, c.syntax));
        ranked.truncate(top);
        render(selection, analysis, cost, files, ranked.iter().copied(), json);
    } else {
        render(selection, analysis, cost, files, contributors, json);
    }
    complete
}

fn render<'a>(
    selection: &Selection,
    analysis: &Analysis<'_>,
    cost: Option<usize>,
    files: impl Iterator<Item = (&'a Path, &'a FileScore<'a>)> + Clone,
    contributors: impl Iterator<Item = (&'a Path, &'a Contribution)> + Clone,
    json: bool,
) {
    let files = files.filter(|_| cost.is_some()).map(|(path, score)| FileRow {
        path: display_path(path),
        added_lines: score.ranges.as_slice(),
        diff_hunk_complexity_cost: score.cost,
    });
    let contributors = contributors.map(|(path, contributor)| ContributorRow { path: display_path(path), contributor });
    if json {
        print_json(selection, analysis, Report {
            score_version: VERSION,
            diff_hunk_complexity_cost: cost,
            files: Rows(files),
            contributors: Rows(contributors),
        });
        return;
    }
    print_summary(selection, analysis, "Diff-hunk Scorecard");
    println!("Score version: {VERSION}");
    let Some(cost) = cost else {
        println!("No comparable score.");
        return;
    };
    println!("Diff-hunk complexity cost: {cost}");
    for file in files {
        print!("{}: cost {}; new-side lines:", file.path, file.diff_hunk_complexity_cost);
        if file.added_lines.is_empty() {
            print!(" none");
        }
        for range in file.added_lines {
            print!(" {}-{}", range.start, range.end);
        }
        println!();
    }
    for row in contributors {
        let c = row.contributor;
        println!("{}:{}:{}: {} ({}) +{}", row.path, c.line, c.column, c.kind.label(), c.syntax, c.cost);
    }
}
