mod extract;
mod includes;
mod metrics;
mod report;
mod score;
mod signature;

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Serialize;
use tree_sitter::{Node, Parser};

use crate::discovery::cpp::{display_path, Selection};
use metrics::{BodyMetrics, Loc};
use signature::Signature;

pub(crate) use report::{explain_loc, run, Report};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Namespace,
    Class,
    Struct,
    Union,
    Enum,
    Enumerator,
    Alias,
    Concept,
    Variable,
    Field,
    Function,
    Method,
    Constructor,
    Destructor,
    Operator,
    ConversionOperator,
    Lambda,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Namespace => "namespace",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Union => "union",
            Self::Enum => "enum",
            Self::Enumerator => "enumerator",
            Self::Alias => "alias",
            Self::Concept => "concept",
            Self::Variable => "variable",
            Self::Field => "field",
            Self::Function => "function",
            Self::Method => "method",
            Self::Constructor => "constructor",
            Self::Destructor => "destructor",
            Self::Operator => "operator",
            Self::ConversionOperator => "conversion_operator",
            Self::Lambda => "lambda",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Access {
    Public,
    Protected,
    Private,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    Declaration,
    Definition,
}

impl Role {
    fn label(self) -> &'static str {
        match self {
            Self::Declaration => "declaration",
            Self::Definition => "definition",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
struct Span {
    byte_start: usize,
    byte_end: usize,
    line_start: usize,
    line_end: usize,
}

impl Span {
    fn through(first: Node<'_>, last: Node<'_>) -> Self {
        let end = last.end_position();
        let line_start = first.start_position().row + 1;
        Self {
            byte_start: first.start_byte(),
            byte_end: last.end_byte(),
            line_start,
            line_end: (end.row + usize::from(end.column != 0)).max(line_start),
        }
    }

    fn of(node: Node<'_>) -> Self {
        Self::through(node, node)
    }

    fn lines(self) -> usize {
        self.line_end - self.line_start + 1
    }
}

struct Body {
    span: Span,
    metrics: BodyMetrics,
}

struct Symbol {
    kind: Kind,
    role: Role,
    name: String,
    span: Span,
    access: Option<Access>,
    named_surface: bool,
    template_heads: usize,
    template_parameters: usize,
    signature: Option<Signature>,
    body: Option<Body>,
    qualified_owner: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OwnerKind {
    Namespace,
    Record,
    Ambiguous,
}

impl OwnerKind {
    fn observe(scopes: &mut HashMap<String, Self>, name: String, kind: Self) {
        scopes.entry(name).and_modify(|seen| {
            if *seen != kind {
                *seen = Self::Ambiguous;
            }
        }).or_insert(kind);
    }
}

impl Symbol {
    fn resolve_owner(&mut self, kind: OwnerKind) {
        match kind {
            OwnerKind::Namespace => self.access = None,
            OwnerKind::Record if self.kind == Kind::Function => self.kind = Kind::Method,
            _ => {}
        }
    }
}

struct File<'a> {
    path: &'a Path,
    header: bool,
    loc: Option<Loc>,
    symbols: Vec<Symbol>,
    includes: Vec<includes::Directive>,
}

#[derive(Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Complete,
    Incomplete,
    Empty,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Incomplete => "incomplete",
            Self::Empty => "empty",
        }
    }
}

#[derive(PartialEq, Eq, Serialize)]
struct Diagnostic {
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    column: Option<usize>,
    kind: &'static str,
    message: String,
}

impl Diagnostic {
    fn file(path: &Path, kind: &'static str, message: String) -> Self {
        Self { file: display_path(path).into_owned(), line: None, column: None, kind, message }
    }

    fn at(path: &Path, node: Node<'_>, kind: &'static str, message: String) -> Self {
        let pos = node.start_position();
        Self {
            file: display_path(path).into_owned(),
            line: Some(pos.row + 1),
            column: Some(pos.column + 1),
            kind,
            message,
        }
    }
}

struct Analysis<'a> {
    files: Vec<File<'a>>,
    diagnostics: Vec<Diagnostic>,
    parsed: usize,
    status: Status,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Inputs {
    Loc,
    Symbols,
    Scorecard,
}

fn analyze(selection: &Selection, inputs: Inputs) -> Analysis<'_> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_cpp::LANGUAGE.into())
        .expect("pinned Tree-sitter C++ grammar ABI");
    let mut analysis = Analysis {
        files: Vec::with_capacity(selection.files.len()),
        diagnostics: selection
            .diagnostics
            .iter()
            .map(|d| Diagnostic::file(&d.file, "discovery_error", d.message.clone()))
            .collect(),
        parsed: 0,
        status: Status::Complete,
    };
    let mut scopes = HashMap::new();
    let mut local_scopes = HashMap::new();
    for path in &selection.files {
        let source = match fs::read(selection.root.join(path)) {
            Ok(source) => source,
            Err(error) => {
                analysis.diagnostics.push(Diagnostic::file(path, "read_error", error.to_string()));
                continue;
            }
        };
        let Some(tree) = parser.parse(&source, None) else {
            analysis.diagnostics.push(Diagnostic::file(path, "parse_error", "parser returned no tree".into()));
            parser.reset();
            continue;
        };
        let root = tree.root_node();
        if root.has_error() {
            syntax_diagnostics(path, root, &mut analysis.diagnostics);
        } else {
            analysis.parsed += 1;
        }
        analysis.files.push(File {
            path,
            header: inputs != Inputs::Loc
                && path.extension().is_some_and(|ext| {
                    matches!(ext.to_str(), Some("h" | "hpp" | "hh" | "hxx" | "ipp" | "tpp"))
                }),
            loc: (inputs != Inputs::Symbols).then(|| metrics::physical_loc(&source, root)),
            symbols: if inputs == Inputs::Loc {
                Vec::new()
            } else {
                extract::symbols(root, &source, &mut local_scopes)
            },
            includes: if inputs == Inputs::Scorecard {
                includes::collect(&source, root)
            } else {
                Vec::new()
            },
        });
        for (name, kind) in local_scopes.drain() {
            OwnerKind::observe(&mut scopes, name, kind);
        }
    }
    for file in &mut analysis.files {
        for symbol in &mut file.symbols {
            if let Some(kind) = symbol.qualified_owner.take().and_then(|name| scopes.get(&name).copied()) {
                symbol.resolve_owner(kind);
            }
        }
    }
    analysis.finish(selection.files.len());
    analysis
}

impl Analysis<'_> {
    fn finish(&mut self, selected: usize) {
        self.diagnostics.sort_unstable_by(|a, b| {
            (&a.file, a.line, a.column, a.kind, &a.message)
                .cmp(&(&b.file, b.line, b.column, b.kind, &b.message))
        });
        // Error recovery can nest errors with identical rendered locations.
        self.diagnostics.dedup();
        self.status = if self.parsed != selected || !self.diagnostics.is_empty() {
            Status::Incomplete
        } else if selected == 0 {
            Status::Empty
        } else {
            Status::Complete
        };
    }
}

fn syntax_diagnostics(path: &Path, root: Node<'_>, diagnostics: &mut Vec<Diagnostic>) {
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        if node.is_missing() {
            diagnostics.push(Diagnostic::at(path, node, "missing", format!("missing {}", node.kind())));
        } else if node.is_error() {
            diagnostics.push(Diagnostic::at(path, node, "syntax_error", "unexpected syntax".into()));
        }
        if node.has_error() && cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}
