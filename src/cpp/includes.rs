use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::{Serialize, Serializer};
use tree_sitter::Node;

use super::{Diagnostic, File, Span};
use crate::discovery::cpp::{display_path, include_hit, FileIdentity, IncludeHit, IncludeUniverse, Selection};

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Form {
    Quoted,
    Angle,
    Macro,
}

impl Form {
    fn label(self) -> &'static str {
        match self {
            Self::Quoted => "quoted",
            Self::Angle => "angle",
            Self::Macro => "macro",
        }
    }
}

pub(super) struct Directive {
    span: Span,
    column: usize,
    form: Form,
    spelling: Vec<u8>,
}

pub(super) fn collect(source: &[u8], root: Node<'_>) -> Vec<Directive> {
    let mut directives = Vec::new();
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        if node.kind() == "preproc_include" {
            let (form, spelling) = match node.child_by_field_name("path") {
                Some(path) => {
                    let bytes = &source[path.byte_range()];
                    match (path.kind(), bytes) {
                        ("string_literal", [b'"', spelling @ .., b'"']) => (Form::Quoted, spelling),
                        ("system_lib_string", [b'<', spelling @ .., b'>']) => (Form::Angle, spelling),
                        _ => (Form::Macro, bytes),
                    }
                }
                None => (Form::Macro, &source[node.byte_range()]),
            };
            directives.push(Directive {
                span: Span::of(node),
                column: node.start_position().column + 1,
                form,
                spelling: spelling.to_vec(),
            });
        }
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return directives;
            }
        }
    }
}

#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct FileCoupling {
    pub(super) fan_in: usize,
    pub(super) fan_out: usize,
    pub(super) cyclic_edges: usize,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Internal,
    Ambiguous,
    External,
    Unresolved,
    Macro,
}

impl Outcome {
    fn label(self) -> &'static str {
        match self {
            Self::Internal => "internal",
            Self::Ambiguous => "ambiguous",
            Self::External => "external",
            Self::Unresolved => "unresolved",
            Self::Macro => "macro",
        }
    }
}

#[derive(Serialize)]
struct DirectiveReport {
    source: String,
    span: Span,
    column: usize,
    form: Form,
    spelling: String,
    status: Outcome,
    target: Option<String>,
    candidates: Vec<String>,
}

#[derive(Serialize)]
struct NodeReport {
    #[serde(serialize_with = "serialize_path")]
    path: PathBuf,
    #[serde(flatten)]
    metrics: FileCoupling,
}

#[derive(Serialize)]
struct EdgeReport {
    source: String,
    target: String,
    cyclic: bool,
}

#[derive(Serialize)]
pub(super) struct Graph {
    directives: Vec<DirectiveReport>,
    nodes: Vec<NodeReport>,
    edges: Vec<EdgeReport>,
    cyclic_components: Vec<Vec<String>>,
}

impl Graph {
    pub(super) fn build(selection: &Selection, files: &[File<'_>]) -> Result<Self, Vec<Diagnostic>> {
        let mut universe = None;
        let mut representatives: HashMap<FileIdentity, PathBuf> = HashMap::new();
        let mut pending = Vec::new();
        let mut diagnostics = Vec::new();
        for file in files {
            for directive in &file.includes {
                let spelling = display_bytes(&directive.spelling);
                let mut row = DirectiveReport {
                    source: display_path(file.path).into_owned(),
                    span: directive.span,
                    column: directive.column,
                    form: directive.form,
                    spelling,
                    status: Outcome::Macro,
                    target: None,
                    candidates: Vec::new(),
                };
                let mut identity = None;
                if !matches!(directive.form, Form::Macro) {
                    let hits = match search(selection, file.path, directive) {
                        Ok(hits) => hits,
                        Err(errors) => {
                            diagnostics.extend(errors.into_iter().map(|message| {
                                diagnostic(file.path, directive, message)
                            }));
                            continue;
                        }
                    };
                    // Sorted paths within each physical group; a single representative
                    // per hit for reporting, even when configured roots overlap.
                    let mut candidate_paths: Vec<_> = hits.values().map(|group| &group[0].0).collect();
                    candidate_paths.sort_unstable();
                    row.candidates.extend(candidate_paths.into_iter().map(|path| display_path(path).into_owned()));
                    row.status = match hits.len() {
                        0 => Outcome::Unresolved,
                        1 => {
                            let (key, group) = hits.into_iter().next().unwrap();
                            if universe.is_none() {
                                match IncludeUniverse::new(selection) {
                                    Ok(value) => universe = Some(value),
                                    Err(message) => {
                                        diagnostics.push(diagnostic(file.path, directive, message));
                                        return Err(diagnostics);
                                    }
                                }
                            }
                            let universe = universe.as_mut().unwrap();
                            let mut eligible = None;
                            for (spelled, hit) in &group {
                                match universe.eligible(spelled, hit) {
                                    Ok(Some(path)) if eligible.as_ref().is_none_or(|best| &path < best) => {
                                        eligible = Some(path);
                                    }
                                    Ok(_) => {}
                                    Err(error) => diagnostics.push(diagnostic(
                                        file.path,
                                        directive,
                                        format!("Cannot determine include target eligibility for {}: {error}", display_path(spelled)),
                                    )),
                                }
                            }
                            if let Some(path) = eligible {
                                let path = universe.selected_path(&key).map_or(path, Path::to_path_buf);
                                representatives.entry(key.clone()).and_modify(|best| {
                                    if path < *best {
                                        best.clone_from(&path);
                                    }
                                }).or_insert(path);
                                identity = Some(key);
                                Outcome::Internal
                            } else {
                                Outcome::External
                            }
                        }
                        _ => Outcome::Ambiguous,
                    };
                }
                pending.push((file.path, row, identity));
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }

        let mut paths: Vec<&Path> = selection.files.iter().map(PathBuf::as_path).collect();
        paths.extend(representatives.values().map(PathBuf::as_path));
        paths.sort_unstable();
        paths.dedup();
        let indices: HashMap<_, _> = paths.iter().copied().enumerate().map(|(i, path)| (path, i)).collect();
        let names: Vec<_> = paths.iter().map(|path| display_path(path)).collect();
        let mut pairs = BTreeSet::new();
        pending.sort_unstable_by_key(|(path, row, _)| (*path, row.span.byte_start, row.span.byte_end));
        let directives = pending.into_iter().map(|(source, mut row, identity)| {
            if let Some(identity) = identity {
                let target = indices[representatives[&identity].as_path()];
                pairs.insert((indices[source], target));
                row.target = Some(names[target].to_string());
            }
            row
        }).collect();

        let (groups, group_ids) = components(paths.len(), &pairs);
        let mut nodes: Vec<_> = paths.into_iter().map(|path| NodeReport {
            path: path.to_path_buf(),
            metrics: FileCoupling::default(),
        }).collect();
        let mut self_edges = vec![false; nodes.len()];
        let edges = pairs.into_iter().map(|(source, target)| {
            let cyclic = group_ids[source] == group_ids[target];
            nodes[source].metrics.fan_out += 1;
            nodes[source].metrics.cyclic_edges += usize::from(cyclic);
            nodes[target].metrics.fan_in += 1;
            self_edges[source] |= source == target;
            EdgeReport { source: names[source].to_string(), target: names[target].to_string(), cyclic }
        }).collect();
        let mut cyclic_groups: Vec<_> = groups.into_iter()
            .filter(|group| group.len() > 1 || self_edges[group[0]])
            .collect();
        cyclic_groups.sort_unstable();
        let cyclic_components = cyclic_groups.into_iter()
            .map(|group| group.into_iter().map(|node| names[node].to_string()).collect())
            .collect();
        Ok(Self { directives, nodes, edges, cyclic_components })
    }

    pub(super) fn file_metrics(&self, path: &Path) -> FileCoupling {
        self.nodes.binary_search_by(|node| node.path.as_path().cmp(path))
            .map(|index| self.nodes[index].metrics)
            .unwrap_or_default()
    }

    pub(super) fn print(&self) {
        println!("Includes: {} directive(s), {} internal edge(s), {} cyclic component(s)", self.directives.len(), self.edges.len(), self.cyclic_components.len());
        for row in &self.directives {
            print!(
                "  {:?}:{}:{} (through line {}, bytes {}..{}) {} {:?}: {}",
                row.source, row.span.line_start, row.column, row.span.line_end,
                row.span.byte_start, row.span.byte_end, row.form.label(), row.spelling, row.status.label(),
            );
            if let Some(target) = &row.target {
                print!(" -> {target:?}");
            }
            if !row.candidates.is_empty() {
                print!("; candidates: {:?}", row.candidates);
            }
            println!();
        }
        for node in &self.nodes {
            let FileCoupling { fan_in, fan_out, cyclic_edges } = node.metrics;
            println!("  Node {:?}: fan-in {fan_in}, fan-out {fan_out}, cyclic edges {cyclic_edges}", display_path(&node.path));
        }
        for edge in &self.edges {
            println!("  Edge {:?} -> {:?}{}", edge.source, edge.target, if edge.cyclic { " (cyclic)" } else { "" });
        }
        for group in &self.cyclic_components {
            println!("  Cyclic component: {group:?}");
        }
    }
}

fn diagnostic(source: &Path, directive: &Directive, message: String) -> Diagnostic {
    Diagnostic {
        file: display_path(source).into_owned(),
        line: Some(directive.span.line_start),
        column: Some(directive.column),
        kind: "include_resolution_error",
        message,
    }
}

type Hits = HashMap<FileIdentity, Vec<(PathBuf, IncludeHit)>>;

fn search(selection: &Selection, source: &Path, directive: &Directive) -> Result<Hits, Vec<String>> {
    let mut hits = Hits::new();
    let name = join_spliced_lines(&directive.spelling);
    let Some(spelling) = spelled_path(&name) else {
        return Ok(hits);
    };
    if spelling.as_os_str().is_empty() || spelling.has_root() || matches!(spelling.components().next(), Some(std::path::Component::Prefix(_))) {
        return Ok(hits);
    }
    let mut errors = Vec::new();
    let mut probe = |base: &Path| {
        let path = base.join(spelling);
        match include_hit(&path) {
            Ok(Some(hit)) => {
                hits.entry(hit.identity.clone()).or_default().push((path, hit));
                true
            }
            Ok(None) => false,
            Err(error) => {
                errors.push(format!("Cannot inspect include candidate {}: {error}", display_path(&path)));
                true
            }
        }
    };
    if !matches!(directive.form, Form::Quoted)
        || !probe(&selection.root.join(source.parent().unwrap()))
    {
        for root in &selection.include_dirs {
            probe(root);
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    for group in hits.values_mut() {
        group.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
    }
    Ok(hits)
}

// A header name is spelled after C++ line splicing. Preserve the source token
// for reporting; remove the splices only for lookup.
fn join_spliced_lines(bytes: &[u8]) -> Cow<'_, [u8]> {
    let mut joined = Vec::new();
    let mut start = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        if byte != b'\\' {
            continue;
        }
        let width = match &bytes[index + 1..] {
            [b'\n', ..] => 2,
            [b'\r', b'\n', ..] => 3,
            _ => continue,
        };
        if start == 0 {
            joined.reserve_exact(bytes.len() - width);
        }
        joined.extend_from_slice(&bytes[start..index]);
        start = index + width;
    }
    if start == 0 {
        Cow::Borrowed(bytes)
    } else {
        joined.extend_from_slice(&bytes[start..]);
        Cow::Owned(joined)
    }
}

fn spelled_path(bytes: &[u8]) -> Option<&Path> {
    if bytes.contains(&0) {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Some(Path::new(std::ffi::OsStr::from_bytes(bytes)))
    }
    #[cfg(not(unix))]
    {
        std::str::from_utf8(bytes).ok().map(Path::new)
    }
}

fn display_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().flat_map(|&byte| std::ascii::escape_default(byte)).map(char::from).collect(),
    }
}

fn serialize_path<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&display_path(path))
}

// Kosaraju with iterative traversals: include chains can be much deeper than
// the call stack. Member and edge ordering come from the sorted node indices.
fn components(count: usize, pairs: &BTreeSet<(usize, usize)>) -> (Vec<Vec<usize>>, Vec<usize>) {
    let mut outgoing = vec![Vec::new(); count];
    let mut incoming = vec![Vec::new(); count];
    for &(source, target) in pairs {
        outgoing[source].push(target);
        incoming[target].push(source);
    }
    let mut visited = vec![false; count];
    let mut finish = Vec::with_capacity(count);
    let mut frames = Vec::new();
    for start in 0..count {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        frames.push((start, outgoing[start].iter()));
        while let Some((node, edges)) = frames.last_mut() {
            if let Some(&target) = edges.next() {
                if !visited[target] {
                    visited[target] = true;
                    frames.push((target, outgoing[target].iter()));
                }
            } else {
                finish.push(*node);
                frames.pop();
            }
        }
    }
    let mut groups = Vec::new();
    let mut ids = vec![usize::MAX; count];
    let mut stack = Vec::new();
    for start in finish.into_iter().rev() {
        if ids[start] != usize::MAX {
            continue;
        }
        let mut group = Vec::new();
        ids[start] = groups.len();
        stack.push(start);
        while let Some(node) = stack.pop() {
            group.push(node);
            for &target in &incoming[node] {
                if ids[target] == usize::MAX {
                    ids[target] = groups.len();
                    stack.push(target);
                }
            }
        }
        group.sort_unstable();
        groups.push(group);
    }
    (groups, ids)
}
