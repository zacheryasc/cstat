//! Shared lazy build of every graph that Stage 9 levels read.
//!
//! `syn::File` is `!Send + !Sync`, so this cache parses + walks every `.rs`
//! file under the project root and discards the parsed AST before returning;
//! the cached value is plain numeric / string data. The existing
//! [`SharedAst`] cache covers the metric-extraction walk; this cache adds
//! the graph-shaped facts (use edges, call edges, per-function CFG stats,
//! type-usage edges) and keeps them in one place so every Stage 9 level
//! reads the same source of truth.
//!
//! Graph sizes are bounded by source size, so we hold everything in plain
//! `Vec`s and identify nodes by their index. Levels project this onto
//! whichever graph they need.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, OnceLock};

use syn::visit::{self, Visit};

use crate::ast_cache::SharedAst;
use crate::inventory::SharedInventory;

/// Per-function basic-block + edge counts approximated from the syn body.
///
/// "Approximated" because we don't lower to MIR — we treat each top-level
/// linear statement run as one basic block and bump the node / edge count
/// for each branching construct (`if`, `match`, `while`, `loop`, `for`,
/// short-circuit `&&` / `||`, `?`).
#[derive(Debug, Clone)]
pub(crate) struct CfgStats {
    /// Index into [`GraphData::functions`] for the function this CFG
    /// describes. Retained so future cross-level joins (e.g. level 35
    /// dataflow density per fn) don't have to re-resolve by fq_path.
    #[allow(dead_code)]
    pub function_index: usize,
    pub node_count: u64,
    pub edge_count: u64,
    pub branch_count: u64,
    pub return_count: u64,
}

/// One node in the intra-crate call graph (level 32). The index of this
/// vector is the node id used by `call_edges`.
#[derive(Debug, Clone)]
pub(crate) struct FunctionNode {
    pub fq_path: String,
    /// Short name (without module qualification). Exposed for future
    /// per-function entity records that don't want to re-derive it from
    /// `fq_path`.
    #[allow(dead_code)]
    pub name: String,
    /// Posix path of the file the function was declared in. Exposed so
    /// downstream graph entities can attribute back to a `FileId` without
    /// re-walking the AST cache.
    #[allow(dead_code)]
    pub file_path: String,
    pub module_path: String,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct GraphData {
    /// Module nodes (level 31). One entry per distinct crate-qualified
    /// module path discovered in the project. Indexed by node id.
    pub modules: Vec<String>,
    /// Module-graph edges. `(src, dst)` indices into `modules`. An edge
    /// exists when a file in module `src` contains `use <crate>::dst::…;`.
    /// Self-edges are dropped. Edges are deduplicated.
    pub module_edges: Vec<(usize, usize)>,

    /// Function nodes (level 32). Indexed by node id.
    pub functions: Vec<FunctionNode>,
    /// Call-graph edges. `(caller, callee)` indices into `functions`. A
    /// callee is resolved by short name; if the short name is ambiguous
    /// (multiple defs in the crate) we conservatively drop the edge to
    /// avoid manufacturing spurious links.
    pub call_edges: Vec<(usize, usize)>,

    /// Distinct type names (level 36 / type-usage bipartite). Indexed.
    pub types: Vec<String>,
    /// Bipartite function → type edges (level 36).
    pub type_usage_edges: Vec<(usize, usize)>,

    /// Per-function CFG node / edge counts (level 34). Indexed positionally
    /// against `functions`.
    pub cfg: Vec<CfgStats>,

    /// Per-file aggregate branch counts. Used by level 40 (self-similarity).
    pub file_branch_totals: Vec<u64>,
    /// Per-module aggregate branch counts. Used by level 40.
    pub module_branch_totals: Vec<u64>,
}

#[derive(Clone)]
pub(crate) struct SharedGraphs {
    inv: SharedInventory,
    ast: SharedAst,
    cell: Arc<OnceLock<GraphData>>,
}

impl SharedGraphs {
    pub fn new(inv: SharedInventory, ast: SharedAst) -> Self {
        Self {
            inv,
            ast,
            cell: Arc::new(OnceLock::new()),
        }
    }

    pub fn get(&self, project_root: &Path) -> &GraphData {
        self.cell
            .get_or_init(|| build_graphs(&self.inv, &self.ast, project_root))
    }
}

fn build_graphs(inv: &SharedInventory, ast_cache: &SharedAst, project_root: &Path) -> GraphData {
    let inv = inv.get(project_root);
    let ast = ast_cache.get(project_root);

    // Function nodes come from the AST cache (already a complete enumeration
    // of every fn / impl-fn the project declares). Build an index by short
    // name so we can resolve callees later — if a name is ambiguous, we
    // keep both indices but the resolver drops the edge.
    let mut functions: Vec<FunctionNode> = Vec::new();
    let mut by_short_name: HashMap<String, Vec<usize>> = HashMap::new();
    for fs in ast.iter_functions() {
        let idx = functions.len();
        functions.push(FunctionNode {
            fq_path: fs.fq_path.clone(),
            name: fs.name.clone(),
            file_path: posix(&fs.file_path),
            module_path: crate_qualify(&fs.module_path),
        });
        by_short_name
            .entry(fs.name.clone())
            .or_default()
            .push(idx);
    }

    // Module nodes from the AST cache's module summaries. The cache
    // stores per-file root modules as just the file stem ("helpers" for
    // `src/helpers.rs`); we crate-qualify here so use-path resolution
    // and the §7 ModuleId stamping line up with the spec.
    let mut module_set: BTreeSet<String> = BTreeSet::new();
    for f in &ast.files {
        for m in &f.modules {
            module_set.insert(crate_qualify(&m.module_path));
        }
    }
    for fs in ast.iter_functions() {
        module_set.insert(crate_qualify(&fs.module_path));
    }
    let modules: Vec<String> = module_set.into_iter().collect();
    let module_index: HashMap<String, usize> = modules
        .iter()
        .enumerate()
        .map(|(i, m)| (m.clone(), i))
        .collect();

    // Per-file branch totals & per-module branch totals, plus per-function
    // CFG stats, plus call edges. One re-parse per file is necessary
    // because the existing AST cache discards the parsed `syn::File`.
    let mut call_edge_set: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut module_edge_set: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut cfg_by_fq: HashMap<String, CfgStats> = HashMap::new();
    let mut per_file_branches: BTreeMap<String, u64> = BTreeMap::new();
    let mut per_module_branches: BTreeMap<String, u64> = BTreeMap::new();

    for rf in &inv.rust_files {
        let Some(text) = rf.text.as_deref() else { continue };
        let Ok(file) = syn::parse_file(text) else { continue };

        let file_posix = posix(&rf.relative_path);
        let root_module = file_root_module(&rf.relative_path);

        // Collect uses + per-function call names + per-function CFG.
        let mut walker = FileWalker::new(root_module.clone());
        walker.visit_file(&file);

        // Module edges from `use` clauses.
        let src_module = crate_qualify(&root_module);
        for use_path in &walker.use_paths {
            let Some(target) = resolve_use_to_module(use_path, &module_index) else {
                continue;
            };
            // Source module is the function's containing module if the use
            // is nested inside an inline mod; v1 takes the file's root
            // module for simplicity (the same approximation used for
            // function module-paths in ast_cache).
            let Some(src) = module_index.get(&src_module).copied() else {
                continue;
            };
            if src != target {
                module_edge_set.insert((src, target));
            }
        }

        // Per-function CFG + call-graph edges.
        let mut file_branches = 0u64;
        for fn_walk in walker.functions {
            let fq_path = fn_walk.fq_path.clone();
            let caller_idx = match functions.iter().position(|f| f.fq_path == fq_path) {
                Some(i) => i,
                None => continue, // ast_cache and re-parse disagree; skip
            };

            // Resolve callees: only count edges where the short name has
            // exactly one definition in this crate.
            for callee_name in &fn_walk.call_names {
                if let Some(matches) = by_short_name.get(callee_name) {
                    if matches.len() == 1 {
                        let callee_idx = matches[0];
                        call_edge_set.insert((caller_idx, callee_idx));
                    }
                }
            }

            file_branches += fn_walk.cfg.branch_count;
            *per_module_branches
                .entry(functions[caller_idx].module_path.clone())
                .or_insert(0) += fn_walk.cfg.branch_count;

            cfg_by_fq.insert(
                fq_path,
                CfgStats {
                    function_index: caller_idx,
                    node_count: fn_walk.cfg.node_count,
                    edge_count: fn_walk.cfg.edge_count,
                    branch_count: fn_walk.cfg.branch_count,
                    return_count: fn_walk.cfg.return_count,
                },
            );
        }
        per_file_branches.insert(file_posix, file_branches);
        let _ = root_module;
    }

    // Order CFG records positionally against `functions` so downstream
    // levels can iterate the two together.
    let mut cfg: Vec<CfgStats> = Vec::with_capacity(functions.len());
    for (i, f) in functions.iter().enumerate() {
        let stats = cfg_by_fq
            .remove(&f.fq_path)
            .unwrap_or(CfgStats {
                function_index: i,
                node_count: 1,
                edge_count: 0,
                branch_count: 0,
                return_count: 0,
            });
        cfg.push(stats);
    }

    // Type-usage bipartite: function index → type index.
    let mut type_set: BTreeSet<String> = BTreeSet::new();
    for fs in ast.iter_functions() {
        for t in &fs.type_usage.distinct {
            type_set.insert(t.clone());
        }
    }
    let types: Vec<String> = type_set.into_iter().collect();
    let type_index: HashMap<String, usize> = types
        .iter()
        .enumerate()
        .map(|(i, t)| (t.clone(), i))
        .collect();

    let mut type_usage_edges: BTreeSet<(usize, usize)> = BTreeSet::new();
    for (fn_idx, fs) in ast.iter_functions().enumerate() {
        // `iter_functions()` iterates in the same order that we used to
        // populate `functions` above, so indices line up.
        for t in &fs.type_usage.distinct {
            if let Some(&tidx) = type_index.get(t) {
                type_usage_edges.insert((fn_idx, tidx));
            }
        }
    }

    let mut module_edges: Vec<(usize, usize)> = module_edge_set.into_iter().collect();
    module_edges.sort_unstable();
    let mut call_edges: Vec<(usize, usize)> = call_edge_set.into_iter().collect();
    call_edges.sort_unstable();
    let mut type_usage_edges: Vec<(usize, usize)> = type_usage_edges.into_iter().collect();
    type_usage_edges.sort_unstable();

    let file_branch_totals: Vec<u64> = per_file_branches.into_values().collect();
    let module_branch_totals: Vec<u64> = per_module_branches.into_values().collect();

    GraphData {
        modules,
        module_edges,
        functions,
        call_edges,
        types,
        type_usage_edges,
        cfg,
        file_branch_totals,
        module_branch_totals,
    }
}

// ───── File-level visitor: uses + per-function call names + per-fn CFG ──

struct FileWalker {
    /// Stack of module paths currently being walked. Bottom is the file's
    /// root module; nested `mod` blocks push.
    module_stack: Vec<String>,
    use_paths: Vec<String>,
    functions: Vec<FnWalkResult>,
}

struct FnWalkResult {
    fq_path: String,
    call_names: Vec<String>,
    cfg: CfgWalkStats,
}

#[derive(Default)]
struct CfgWalkStats {
    node_count: u64,
    edge_count: u64,
    branch_count: u64,
    return_count: u64,
}

impl FileWalker {
    fn new(root_module: String) -> Self {
        Self {
            module_stack: vec![root_module],
            use_paths: Vec::new(),
            functions: Vec::new(),
        }
    }

    fn current_module(&self) -> &str {
        self.module_stack.last().expect("module stack non-empty")
    }

    fn record_function(&mut self, ident: &syn::Ident, body: &syn::Block) {
        let fq_path = format!("{}::{}", self.current_module(), ident);
        let mut fn_visitor = FnBodyVisitor::default();
        fn_visitor.visit_block(body);
        // Linear baseline: one entry basic block. Every branching construct
        // adds two nodes (then / else or merge) and two edges; each `match`
        // arm beyond the first adds one. Return / break / continue add a
        // post-jump node + edge. This intentionally undercounts vs. a
        // real MIR-derived CFG, but tracks within a constant factor.
        let mut cfg = fn_visitor.cfg;
        cfg.node_count = cfg.node_count.saturating_add(1); // entry block
        cfg.edge_count = cfg.edge_count.saturating_add(cfg.branch_count); // forward edges

        self.functions.push(FnWalkResult {
            fq_path,
            call_names: fn_visitor.call_names,
            cfg,
        });
    }
}

impl<'ast> Visit<'ast> for FileWalker {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        match item {
            syn::Item::Use(item_use) => {
                collect_use_paths(&item_use.tree, &mut Vec::new(), &mut self.use_paths);
            }
            syn::Item::Fn(item_fn) => {
                self.record_function(&item_fn.sig.ident, &item_fn.block);
            }
            syn::Item::Mod(item_mod) => {
                if let Some((_, items)) = &item_mod.content {
                    let parent = self.current_module().to_string();
                    self.module_stack
                        .push(format!("{parent}::{}", item_mod.ident));
                    for inner in items {
                        self.visit_item(inner);
                    }
                    self.module_stack.pop();
                }
                // External `mod foo;` declarations are walked when their
                // file shows up in the inventory.
                return;
            }
            syn::Item::Impl(item_impl) => {
                for assoc in &item_impl.items {
                    if let syn::ImplItem::Fn(impl_fn) = assoc {
                        self.record_function(&impl_fn.sig.ident, &impl_fn.block);
                    }
                }
            }
            _ => {}
        }
        visit::visit_item(self, item);
    }
}

/// `use a::{b::c, d as e};` → emits "a::b::c" and "a::d".
fn collect_use_paths(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<String>) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            collect_use_paths(&p.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let mut full = prefix.clone();
            full.push(n.ident.to_string());
            out.push(full.join("::"));
        }
        syn::UseTree::Rename(r) => {
            let mut full = prefix.clone();
            full.push(r.ident.to_string());
            out.push(full.join("::"));
        }
        syn::UseTree::Glob(_) => {
            // `use a::*;` — record the prefix as the imported target.
            if !prefix.is_empty() {
                out.push(prefix.join("::"));
            }
        }
        syn::UseTree::Group(g) => {
            for inner in &g.items {
                collect_use_paths(inner, prefix, out);
            }
        }
    }
}

/// Map a `use` path string to a module-index target, or `None` if the
/// path doesn't resolve to a known crate-local module. Recognized roots:
/// `crate`, `self`, `super`. v1 doesn't try to resolve `super::` against
/// the source module's parent — it treats it as unresolved.
fn resolve_use_to_module(
    use_path: &str,
    module_index: &HashMap<String, usize>,
) -> Option<usize> {
    let path = use_path.strip_prefix("crate::").or_else(|| {
        use_path.strip_prefix("self::")
    });
    let path = path?;

    // Walk prefixes longest → shortest, returning the first one that
    // matches a known module. This handles `use crate::foo::bar::Baz;`
    // when `crate::foo::bar` is a known module but the trailing `Baz` is
    // an item inside it.
    let segments: Vec<&str> = path.split("::").collect();
    for take in (1..=segments.len()).rev() {
        let candidate = format!("crate::{}", segments[..take].join("::"));
        if let Some(&idx) = module_index.get(&candidate) {
            return Some(idx);
        }
    }
    None
}

#[derive(Default)]
struct FnBodyVisitor {
    call_names: Vec<String>,
    cfg: CfgWalkStats,
}

impl FnBodyVisitor {
    fn bump_branch(&mut self) {
        self.cfg.branch_count += 1;
        self.cfg.node_count += 1;
    }
}

impl<'ast> Visit<'ast> for FnBodyVisitor {
    fn visit_expr_call(&mut self, expr: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*expr.func {
            if let Some(last) = path.path.segments.last() {
                self.call_names.push(last.ident.to_string());
            }
        }
        visit::visit_expr_call(self, expr);
    }
    fn visit_expr_method_call(&mut self, expr: &'ast syn::ExprMethodCall) {
        self.call_names.push(expr.method.to_string());
        visit::visit_expr_method_call(self, expr);
    }
    fn visit_expr_if(&mut self, expr: &'ast syn::ExprIf) {
        self.bump_branch();
        if expr.else_branch.is_some() {
            self.bump_branch();
        }
        visit::visit_expr_if(self, expr);
    }
    fn visit_expr_match(&mut self, expr: &'ast syn::ExprMatch) {
        // Each arm is a branch target.
        let arms = expr.arms.len();
        for _ in 0..arms.max(1) {
            self.bump_branch();
        }
        visit::visit_expr_match(self, expr);
    }
    fn visit_expr_while(&mut self, expr: &'ast syn::ExprWhile) {
        self.bump_branch();
        visit::visit_expr_while(self, expr);
    }
    fn visit_expr_for_loop(&mut self, expr: &'ast syn::ExprForLoop) {
        self.bump_branch();
        visit::visit_expr_for_loop(self, expr);
    }
    fn visit_expr_loop(&mut self, expr: &'ast syn::ExprLoop) {
        self.bump_branch();
        visit::visit_expr_loop(self, expr);
    }
    fn visit_expr_try(&mut self, expr: &'ast syn::ExprTry) {
        // `?` short-circuits.
        self.bump_branch();
        visit::visit_expr_try(self, expr);
    }
    fn visit_expr_binary(&mut self, expr: &'ast syn::ExprBinary) {
        if matches!(expr.op, syn::BinOp::And(_) | syn::BinOp::Or(_)) {
            self.bump_branch();
        }
        visit::visit_expr_binary(self, expr);
    }
    fn visit_expr_return(&mut self, expr: &'ast syn::ExprReturn) {
        self.cfg.return_count += 1;
        visit::visit_expr_return(self, expr);
    }
    fn visit_expr_break(&mut self, expr: &'ast syn::ExprBreak) {
        self.bump_branch();
        visit::visit_expr_break(self, expr);
    }
    fn visit_expr_continue(&mut self, expr: &'ast syn::ExprContinue) {
        self.bump_branch();
        visit::visit_expr_continue(self, expr);
    }
}

/// Convert an `ast_cache` module path string to its crate-qualified
/// form. `ast_cache` uses the file stem as the root for non-`lib`/`main`
/// files (e.g. `src/helpers.rs` → `helpers`), so a use-clause like
/// `use crate::helpers;` won't match without normalization. `crate` and
/// anything already prefixed with `crate::` pass through.
fn crate_qualify(module_path: &str) -> String {
    if module_path == "crate" || module_path.starts_with("crate::") {
        module_path.to_string()
    } else {
        format!("crate::{module_path}")
    }
}

fn file_root_module(path: &Path) -> String {
    // Mirror ast_cache's convention: `src/lib.rs` / `src/main.rs` are the
    // crate root; everything else uses the file stem.
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    if stem == "lib" || stem == "main" {
        "crate".to_string()
    } else {
        stem.to_string()
    }
}

fn posix(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_use_paths_flattens_nested_groups_and_renames() {
        let tree: syn::UseTree =
            syn::parse_str("a::{b::c, d as e, f::*}").unwrap();
        let mut out = Vec::new();
        collect_use_paths(&tree, &mut Vec::new(), &mut out);
        out.sort();
        assert_eq!(out, vec!["a::b::c", "a::d", "a::f"]);
    }

    #[test]
    fn resolve_use_to_module_matches_longest_known_prefix() {
        let mut module_index = HashMap::new();
        module_index.insert("crate".to_string(), 0);
        module_index.insert("crate::foo".to_string(), 1);
        module_index.insert("crate::foo::bar".to_string(), 2);

        // Exact module.
        assert_eq!(
            resolve_use_to_module("crate::foo::bar", &module_index),
            Some(2)
        );
        // Item inside a module — falls back to the longest known prefix.
        assert_eq!(
            resolve_use_to_module("crate::foo::bar::Item", &module_index),
            Some(2)
        );
        // Sibling crate's name — not in the local module set.
        assert_eq!(
            resolve_use_to_module("std::fmt::Display", &module_index),
            None
        );
        // `self::` is treated the same as `crate::` in v1: stripped and
        // looked up against the crate-rooted module map. Good enough for
        // crate-root files; refinements wait for proper module-stack
        // tracking inside the file walker.
        assert_eq!(
            resolve_use_to_module("self::foo", &module_index),
            Some(1),
            "self::foo at crate root resolves to crate::foo (v1 approximation)"
        );
    }

    #[test]
    fn fn_body_visitor_counts_branches_and_call_names() {
        let item: syn::ItemFn = syn::parse_str(
            r#"
            fn f(x: i32) -> i32 {
                if x > 0 {
                    g(x)
                } else {
                    h(x).abs()
                }
            }
            "#,
        )
        .unwrap();
        let mut v = FnBodyVisitor::default();
        v.visit_block(&item.block);
        // One `if` with an else branch → 2 branches.
        assert_eq!(v.cfg.branch_count, 2);
        // `g`, `h`, `abs` — three call-like operations.
        let mut names = v.call_names.clone();
        names.sort();
        assert_eq!(names, vec!["abs", "g", "h"]);
    }
}
