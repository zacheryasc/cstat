use colored::Colorize;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{BinOp, Block, Expr, ExprPath, FnArg, ImplItem, Item, Pat, Stmt};

use crate::render;

const DISPLAY_FUNCTION_LIMIT: usize = 8;
const DISPLAY_CLUSTER_LIMIT: usize = 10;

#[derive(Debug, Clone, Serialize)]
pub struct ClusterReport {
    pub files: Vec<FileClusterReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileClusterReport {
    pub path: String,
    pub functions: Vec<FunctionClusterReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionClusterReport {
    pub name: String,
    pub line_start: usize,
    pub line_end: usize,
    pub is_test: bool,
    pub reference_count: usize,
    pub mutation_count: usize,
    pub clusters: Vec<ValueCluster>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValueCluster {
    pub name: String,
    pub reference_lines: Vec<usize>,
    pub mutation_lines: Vec<usize>,
    pub connected_values: Vec<String>,
    #[serde(skip)]
    sort_line: usize,
}

#[derive(Debug, Clone)]
struct StatementAccess {
    line_start: usize,
    defines: Vec<String>,
    references: BTreeSet<String>,
    mutations: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct WorkingCluster {
    name: String,
    defined_line: usize,
    reference_lines: BTreeSet<usize>,
    mutation_lines: BTreeSet<usize>,
    connected_values: BTreeSet<String>,
}

pub fn analyze_files(rs_files: &[PathBuf], project_path: &Path) -> ClusterReport {
    let mut files = Vec::new();
    for file in rs_files {
        if let Some(report) = analyze_file(file, project_path) {
            files.push(report);
        }
    }
    ClusterReport { files }
}

pub fn analyze_file(path: &Path, project_path: &Path) -> Option<FileClusterReport> {
    let source = fs::read_to_string(path).ok()?;
    let syntax = syn::parse_file(&source).ok()?;
    let mut functions = Vec::new();
    collect_item_functions(&syntax.items, &mut Vec::new(), &mut functions);

    let mut function_reports = functions
        .into_iter()
        .filter_map(|function| analyze_function(function))
        .collect::<Vec<_>>();

    function_reports.sort_by(|a, b| {
        a.is_test
            .cmp(&b.is_test)
            .then_with(|| b.reference_count.cmp(&a.reference_count))
            .then_with(|| b.mutation_count.cmp(&a.mutation_count))
            .then_with(|| a.line_start.cmp(&b.line_start))
            .then_with(|| a.name.cmp(&b.name))
    });

    Some(FileClusterReport {
        path: relative_path_string(project_path, path),
        functions: function_reports,
    })
}

pub fn render_cluster(rs_files: &[PathBuf], project_path: &Path, json: bool, verbose: bool) {
    let report = analyze_files(rs_files, project_path);
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        render_cluster_report(&report, verbose);
    }
}

pub fn render_cluster_file(rs_files: &[PathBuf], project_path: &Path, file: &Path, verbose: bool) {
    let report = analyze_file(file, project_path).unwrap_or_else(|| FileClusterReport {
        path: relative_path_string(project_path, file),
        functions: Vec::new(),
    });
    render_file_cluster_report(&report, verbose);

    // Keep signature parity with sibling modules that receive the full project file set.
    let _ = rs_files;
}

pub fn render_cluster_file_json(rs_files: &[PathBuf], project_path: &Path, file: &Path) {
    let report = analyze_file(file, project_path).unwrap_or_else(|| FileClusterReport {
        path: relative_path_string(project_path, file),
        functions: Vec::new(),
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());

    let _ = rs_files;
}

fn render_cluster_report(report: &ClusterReport, verbose: bool) {
    render::section_header("Value clusters");
    if verbose {
        render_cluster_explanation();
    }

    let mut functions = report
        .files
        .iter()
        .flat_map(|file| {
            file.functions
                .iter()
                .map(move |function| (&file.path, function))
        })
        .collect::<Vec<_>>();
    functions.sort_by(|(path_a, a), (path_b, b)| {
        a.is_test
            .cmp(&b.is_test)
            .then_with(|| b.reference_count.cmp(&a.reference_count))
            .then_with(|| b.mutation_count.cmp(&a.mutation_count))
            .then_with(|| path_a.cmp(path_b))
            .then_with(|| a.line_start.cmp(&b.line_start))
            .then_with(|| a.name.cmp(&b.name))
    });

    if functions.is_empty() {
        println!("No reusable value clusters found.");
        return;
    }

    for (path, function) in functions.into_iter().take(DISPLAY_FUNCTION_LIMIT) {
        println!(
            "{} {}:{}-{}",
            "fn".cyan().bold(),
            path,
            function.line_start,
            function.line_end
        );
        render_function_clusters(function);
    }
}

fn render_file_cluster_report(report: &FileClusterReport, verbose: bool) {
    render::section_header(&format!("Value clusters ({})", report.path));
    if verbose {
        render_cluster_explanation();
    }
    if report.functions.is_empty() {
        println!("No reusable value clusters found.");
        return;
    }

    for function in report.functions.iter().take(DISPLAY_FUNCTION_LIMIT) {
        println!(
            "{} {}:{}-{}",
            "fn".cyan().bold(),
            function.name.bold(),
            function.line_start,
            function.line_end
        );
        render_function_clusters(function);
    }
}

fn render_function_clusters(function: &FunctionClusterReport) {
    println!("  value                     references mutations connected  connected values");
    for cluster in function.clusters.iter().take(DISPLAY_CLUSTER_LIMIT) {
        println!(
            "  {:<24} {:>10} {:>9} {:>9}  {}",
            cluster.name.bold(),
            cluster.reference_lines.len(),
            cluster.mutation_lines.len(),
            cluster.connected_values.len(),
            short_connected_values(&cluster.connected_values, 4).dimmed()
        );
    }

    if function.clusters.len() > DISPLAY_CLUSTER_LIMIT {
        println!(
            "  … {} more values hidden; use --json for the complete cluster record.",
            function.clusters.len() - DISPLAY_CLUSTER_LIMIT
        );
    }
    println!();
}

fn short_connected_values(connected_values: &[String], limit: usize) -> String {
    if connected_values.is_empty() {
        return "-".to_string();
    }
    let mut shown = connected_values
        .iter()
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    if connected_values.len() > limit {
        shown.push(format!("+{} more", connected_values.len() - limit));
    }
    shown.join(", ")
}

fn render_cluster_explanation() {
    render::verbose_line(
        "value = a function parameter or local let-binding collected from the Rust AST.",
    );
    render::verbose_line(
        "references = top-level statements where that value is read after it is created.",
    );
    render::verbose_line(
        "mutations = top-level statements that assign to the value, compound-assign it, or pass it as &mut.",
    );
    render::verbose_line(
        "connected = other local values that appear in the same top-level statements.",
    );
    println!();
}

struct FunctionCandidate<'a> {
    name: String,
    block: &'a Block,
    line_start: usize,
    line_end: usize,
    params: Vec<String>,
    is_test: bool,
}

fn collect_item_functions<'a>(
    items: &'a [Item],
    module_parts: &mut Vec<String>,
    out: &mut Vec<FunctionCandidate<'a>>,
) {
    for item in items {
        match item {
            Item::Fn(function) => {
                let name = qualify_name(module_parts, &function.sig.ident.to_string());
                out.push(FunctionCandidate {
                    name,
                    block: &function.block,
                    line_start: function.sig.fn_token.span.start().line,
                    line_end: function.block.brace_token.span.close().end().line,
                    params: collect_fn_params(&function.sig.inputs),
                    is_test: module_parts.iter().any(|part| part == "tests")
                        || has_attr_named(&function.attrs, "test"),
                });
            }
            Item::Impl(implementation) => {
                let target = type_name(&implementation.self_ty);
                for impl_item in &implementation.items {
                    if let ImplItem::Fn(method) = impl_item {
                        out.push(FunctionCandidate {
                            name: format!("{}::{}", target, method.sig.ident),
                            block: &method.block,
                            line_start: method.sig.fn_token.span.start().line,
                            line_end: method.block.brace_token.span.close().end().line,
                            params: collect_fn_params(&method.sig.inputs),
                            is_test: module_parts.iter().any(|part| part == "tests")
                                || has_attr_named(&method.attrs, "test"),
                        });
                    }
                }
            }
            Item::Mod(module) => {
                if let Some((_, nested)) = &module.content {
                    module_parts.push(module.ident.to_string());
                    collect_item_functions(nested, module_parts, out);
                    module_parts.pop();
                }
            }
            _ => {}
        }
    }
}

fn analyze_function(function: FunctionCandidate<'_>) -> Option<FunctionClusterReport> {
    let mut known_values = BTreeSet::new();
    let mut clusters: BTreeMap<String, WorkingCluster> = BTreeMap::new();

    for param in &function.params {
        if is_usable_binding(param) {
            known_values.insert(param.clone());
            clusters.insert(
                param.clone(),
                WorkingCluster {
                    name: param.clone(),
                    defined_line: function.line_start,
                    reference_lines: BTreeSet::new(),
                    mutation_lines: BTreeSet::new(),
                    connected_values: BTreeSet::new(),
                },
            );
        }
    }

    let mut statements = Vec::new();
    for stmt in &function.block.stmts {
        let line_start = stmt.span().start().line;
        let defines = collect_stmt_definitions(stmt);
        let references = collect_stmt_references(stmt, &known_values);
        let mutations = collect_stmt_mutations(stmt, &known_values);
        for name in &defines {
            if is_usable_binding(name) {
                known_values.insert(name.clone());
            }
        }
        statements.push(StatementAccess {
            line_start,
            defines,
            references,
            mutations,
        });
    }

    for statement in &statements {
        let mut participants = statement.references.clone();
        participants.extend(statement.mutations.iter().cloned());
        for name in &statement.defines {
            if is_usable_binding(name) {
                participants.insert(name.clone());
                clusters
                    .entry(name.clone())
                    .or_insert_with(|| WorkingCluster {
                        name: name.clone(),
                        defined_line: statement.line_start,
                        reference_lines: BTreeSet::new(),
                        mutation_lines: BTreeSet::new(),
                        connected_values: BTreeSet::new(),
                    });
            }
        }

        for name in &participants {
            if let Some(cluster) = clusters.get_mut(name) {
                if statement.references.contains(name) {
                    cluster.reference_lines.insert(statement.line_start);
                }
                if statement.mutations.contains(name) {
                    cluster.mutation_lines.insert(statement.line_start);
                }
                for connected_value in &participants {
                    if connected_value != name && is_usable_binding(connected_value) {
                        cluster.connected_values.insert(connected_value.clone());
                    }
                }
            }
        }
    }

    let mut value_clusters = clusters
        .into_values()
        .filter(|cluster| !cluster.reference_lines.is_empty() || !cluster.mutation_lines.is_empty())
        .map(finalize_cluster)
        .collect::<Vec<_>>();

    value_clusters.sort_by(|a, b| {
        b.reference_lines
            .len()
            .cmp(&a.reference_lines.len())
            .then_with(|| b.mutation_lines.len().cmp(&a.mutation_lines.len()))
            .then_with(|| b.connected_values.len().cmp(&a.connected_values.len()))
            .then_with(|| a.sort_line.cmp(&b.sort_line))
            .then_with(|| a.name.cmp(&b.name))
    });

    if value_clusters.is_empty() {
        return None;
    }

    let reference_count = value_clusters
        .iter()
        .map(|cluster| cluster.reference_lines.len())
        .sum();
    let mutation_count = value_clusters
        .iter()
        .map(|cluster| cluster.mutation_lines.len())
        .sum();
    Some(FunctionClusterReport {
        name: function.name,
        line_start: function.line_start,
        line_end: function.line_end,
        is_test: function.is_test,
        reference_count,
        mutation_count,
        clusters: value_clusters,
    })
}

fn finalize_cluster(cluster: WorkingCluster) -> ValueCluster {
    ValueCluster {
        name: cluster.name,
        reference_lines: cluster.reference_lines.into_iter().collect(),
        mutation_lines: cluster.mutation_lines.into_iter().collect(),
        connected_values: cluster.connected_values.into_iter().collect(),
        sort_line: cluster.defined_line,
    }
}

fn collect_fn_params(
    inputs: &syn::punctuated::Punctuated<FnArg, syn::token::Comma>,
) -> Vec<String> {
    let mut params = Vec::new();
    for input in inputs {
        if let FnArg::Typed(pat_type) = input {
            collect_pat_idents(&pat_type.pat, &mut params);
        }
    }
    params
}

fn collect_stmt_definitions(stmt: &Stmt) -> Vec<String> {
    let mut out = Vec::new();
    if let Stmt::Local(local) = stmt {
        collect_pat_idents(&local.pat, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn collect_stmt_references(stmt: &Stmt, known_values: &BTreeSet<String>) -> BTreeSet<String> {
    let mut collector = ReferenceCollector {
        known_values,
        references: BTreeSet::new(),
    };
    collector.visit_stmt(stmt);
    collector.references
}

fn collect_stmt_mutations(stmt: &Stmt, known_values: &BTreeSet<String>) -> BTreeSet<String> {
    let mut collector = MutationCollector {
        known_values,
        mutations: BTreeSet::new(),
    };
    collector.visit_stmt(stmt);
    collector.mutations
}

struct ReferenceCollector<'a> {
    known_values: &'a BTreeSet<String>,
    references: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for ReferenceCollector<'_> {
    fn visit_expr_path(&mut self, node: &'ast ExprPath) {
        if node.qself.is_none() && node.path.segments.len() == 1 {
            let ident = node.path.segments[0].ident.to_string();
            if self.known_values.contains(&ident) {
                self.references.insert(ident);
            }
        }
        visit::visit_expr_path(self, node);
    }

    fn visit_expr_assign(&mut self, node: &'ast syn::ExprAssign) {
        self.visit_expr(&node.right);
    }
}

struct MutationCollector<'a> {
    known_values: &'a BTreeSet<String>,
    mutations: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for MutationCollector<'_> {
    fn visit_expr_assign(&mut self, node: &'ast syn::ExprAssign) {
        self.record_known_root(&node.left);
        self.visit_expr(&node.right);
    }

    fn visit_expr_binary(&mut self, node: &'ast syn::ExprBinary) {
        if is_assign_binop(&node.op) {
            self.record_known_root(&node.left);
        }
        visit::visit_expr_binary(self, node);
    }

    fn visit_expr_reference(&mut self, node: &'ast syn::ExprReference) {
        if node.mutability.is_some() {
            self.record_known_root(&node.expr);
        }
        visit::visit_expr_reference(self, node);
    }
}

impl MutationCollector<'_> {
    fn record_known_root(&mut self, expr: &Expr) {
        if let Some(name) = root_ident(expr) {
            if self.known_values.contains(&name) {
                self.mutations.insert(name);
            }
        }
    }
}

fn root_ident(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => {
            Some(path.path.segments[0].ident.to_string())
        }
        Expr::Field(field) => root_ident(&field.base),
        Expr::Index(index) => root_ident(&index.expr),
        Expr::Paren(paren) => root_ident(&paren.expr),
        Expr::Group(group) => root_ident(&group.expr),
        Expr::Reference(reference) => root_ident(&reference.expr),
        Expr::Unary(unary) => root_ident(&unary.expr),
        _ => None,
    }
}

fn is_assign_binop(op: &BinOp) -> bool {
    matches!(
        op,
        BinOp::AddAssign(_)
            | BinOp::SubAssign(_)
            | BinOp::MulAssign(_)
            | BinOp::DivAssign(_)
            | BinOp::RemAssign(_)
            | BinOp::BitXorAssign(_)
            | BinOp::BitAndAssign(_)
            | BinOp::BitOrAssign(_)
            | BinOp::ShlAssign(_)
            | BinOp::ShrAssign(_)
    )
}

fn collect_pat_idents(pat: &Pat, out: &mut Vec<String>) {
    match pat {
        Pat::Ident(ident) => {
            let name = ident.ident.to_string();
            if is_usable_binding(&name) {
                out.push(name);
            }
            if let Some((_, subpat)) = &ident.subpat {
                collect_pat_idents(subpat, out);
            }
        }
        Pat::Reference(reference) => collect_pat_idents(&reference.pat, out),
        Pat::Slice(slice) => {
            for elem in &slice.elems {
                collect_pat_idents(elem, out);
            }
        }
        Pat::Struct(strukt) => {
            for field in &strukt.fields {
                collect_pat_idents(&field.pat, out);
            }
        }
        Pat::Tuple(tuple) => {
            for elem in &tuple.elems {
                collect_pat_idents(elem, out);
            }
        }
        Pat::TupleStruct(tuple) => {
            for elem in &tuple.elems {
                collect_pat_idents(elem, out);
            }
        }
        Pat::Type(typed) => collect_pat_idents(&typed.pat, out),
        Pat::Or(or) => {
            for case in &or.cases {
                collect_pat_idents(case, out);
            }
        }
        _ => {}
    }
}

fn is_usable_binding(name: &str) -> bool {
    name != "_" && !name.starts_with('_') && name != "self"
}

fn has_attr_named(attrs: &[syn::Attribute], wanted: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(wanted))
}

fn qualify_name(parts: &[String], name: &str) -> String {
    if parts.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", parts.join("::"), name)
    }
}

fn type_name(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(path) => path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        syn::Type::Reference(reference) => type_name(&reference.elem),
        _ => "impl".to_string(),
    }
}

fn relative_path_string(project_path: &Path, path: &Path) -> String {
    path.strip_prefix(project_path)
        .unwrap_or(path)
        .display()
        .to_string()
}
