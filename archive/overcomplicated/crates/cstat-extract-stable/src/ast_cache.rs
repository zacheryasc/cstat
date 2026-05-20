//! Lazy `syn::parse_file` cache shared by every Group-B level.
//!
//! `syn::File` is `!Send + !Sync` (proc-macro2's stable backend uses `Rc`),
//! so we can't shove parsed ASTs through the orchestrator's `Send + Sync`
//! `Level` trait. Instead this cache parses each `.rs` file once, walks the
//! AST to extract every numeric / textual fact downstream levels need, and
//! discards the parsed `syn::File` before returning. The cached result is
//! plain data — `Send + Sync` falls out for free.
//!
//! Per-file parse failures are non-fatal: the per-file record records the
//! error's line/column and the file is excluded from every downstream
//! metric, per spec §11.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use quote::ToTokens;
use syn::visit::{self, Visit};

use crate::inventory::SharedInventory;

#[derive(Debug, Clone)]
pub(crate) struct FileParseError {
    pub line: usize,
    pub column: usize,
    /// Raw `syn::Error::to_string()` output, retained for diagnostics even
    /// though the entity-record schema is numeric-only today.
    #[allow(dead_code)]
    pub message: String,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct ItemTotals {
    pub functions: u64,
    pub structs: u64,
    pub enums: u64,
    pub traits: u64,
    pub impls: u64,
    pub uses: u64,
    pub consts: u64,
    pub statics: u64,
    pub type_aliases: u64,
    pub pub_items: u64,
    pub pub_crate_items: u64,
    pub private_items: u64,
}

impl ItemTotals {
    pub fn merge(&mut self, other: &ItemTotals) {
        self.functions += other.functions;
        self.structs += other.structs;
        self.enums += other.enums;
        self.traits += other.traits;
        self.impls += other.impls;
        self.uses += other.uses;
        self.consts += other.consts;
        self.statics += other.statics;
        self.type_aliases += other.type_aliases;
        self.pub_items += other.pub_items;
        self.pub_crate_items += other.pub_crate_items;
        self.private_items += other.private_items;
    }

    pub fn item_count(&self) -> u64 {
        self.functions
            + self.structs
            + self.enums
            + self.traits
            + self.impls
            + self.uses
            + self.consts
            + self.statics
            + self.type_aliases
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ModuleSummary {
    pub file_path: PathBuf,
    pub module_path: String,
    pub depth: u32,
    pub totals: ItemTotals,
}

/// One named function item discovered in the AST.
///
/// Carries the bits the DWARF level needs to issue an `AstFuncId` and
/// look up a matching mangled symbol: file, module path, name, and a
/// structural hash of the function body. The hash is FNV-1a over the
/// function's token-stream rendering, which means reformatting leaves
/// the hash stable but real edits change it (per the AstFuncId
/// contract in spec §6).
///
/// `type_usage` records the AST-approximation of spec §2.4 level 16:
/// distinct types referenced in the function's signature and body.
#[derive(Debug, Clone)]
pub(crate) struct FunctionSummary {
    pub file_path: PathBuf,
    /// Module path the function lives in (e.g. `crate::inner`). Not
    /// consumed yet — exposed so a future graph-derived level can
    /// group functions by module without re-walking the AST.
    #[allow(dead_code)]
    pub module_path: String,
    pub name: String,
    pub fq_path: String,
    pub node_hash: String,
    pub type_usage: FunctionTypeUsage,
}

/// Per-function distinct-type counts for spec §2.4 level 16 (AST form).
/// Names are normalized to their last-path-segment form, so unqualified
/// and `use`-imported references count as the same type.
#[derive(Debug, Default, Clone)]
pub(crate) struct FunctionTypeUsage {
    pub distinct: std::collections::BTreeSet<String>,
    pub primitives: std::collections::BTreeSet<String>,
    pub param_distinct: std::collections::BTreeSet<String>,
    pub return_distinct: std::collections::BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct FileAnalysis {
    pub relative_path: PathBuf,
    pub parse_error: Option<FileParseError>,
    pub attribute_count: u64,
    pub macro_invocations: u64,
    /// One entry per item declared in the file, recording its module-nesting
    /// depth. Functions inside `impl` blocks count here too.
    pub item_depths: Vec<u32>,
    pub totals: ItemTotals,
    pub modules: Vec<ModuleSummary>,
    pub functions: Vec<FunctionSummary>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct AstAnalysis {
    pub files: Vec<FileAnalysis>,
}

impl AstAnalysis {
    pub fn attempted(&self) -> usize {
        self.files.len()
    }

    pub fn parsed_ok_count(&self) -> usize {
        self.files.iter().filter(|f| f.parse_error.is_none()).count()
    }

    pub fn iter_errors(&self) -> impl Iterator<Item = (&PathBuf, &FileParseError)> {
        self.files.iter().filter_map(|f| {
            f.parse_error
                .as_ref()
                .map(|e| (&f.relative_path, e))
        })
    }

    /// Flattened iterator over every function summary across all
    /// successfully-parsed files. Stage 7's DWARF level walks this to
    /// build the AST↔Symbol identity join.
    pub fn iter_functions(&self) -> impl Iterator<Item = &FunctionSummary> {
        self.files.iter().flat_map(|f| f.functions.iter())
    }
}

#[derive(Clone)]
pub(crate) struct SharedAst {
    inv: SharedInventory,
    cell: Arc<OnceLock<AstAnalysis>>,
}

impl SharedAst {
    pub fn new(inv: SharedInventory) -> Self {
        Self {
            inv,
            cell: Arc::new(OnceLock::new()),
        }
    }

    pub fn get(&self, project_root: &std::path::Path) -> &AstAnalysis {
        self.cell.get_or_init(|| analyze_all(&self.inv, project_root))
    }
}

fn analyze_all(inv: &SharedInventory, project_root: &std::path::Path) -> AstAnalysis {
    let inv = inv.get(project_root);
    let mut files = Vec::with_capacity(inv.rust_files.len());
    for rf in &inv.rust_files {
        files.push(analyze_one(&rf.relative_path, rf.text.as_deref()));
    }
    AstAnalysis { files }
}

fn analyze_one(relative_path: &std::path::Path, text: Option<&str>) -> FileAnalysis {
    let text = match text {
        Some(t) => t,
        None => {
            return FileAnalysis {
                relative_path: relative_path.to_path_buf(),
                parse_error: Some(FileParseError {
                    line: 0,
                    column: 0,
                    message: "file is not valid UTF-8".into(),
                }),
                attribute_count: 0,
                macro_invocations: 0,
                item_depths: Vec::new(),
                totals: ItemTotals::default(),
                modules: Vec::new(),
                functions: Vec::new(),
            };
        }
    };

    let file = match syn::parse_file(text) {
        Ok(f) => f,
        Err(e) => {
            let span = e.span();
            let start = span.start();
            return FileAnalysis {
                relative_path: relative_path.to_path_buf(),
                parse_error: Some(FileParseError {
                    line: start.line,
                    column: start.column,
                    message: e.to_string(),
                }),
                attribute_count: 0,
                macro_invocations: 0,
                item_depths: Vec::new(),
                totals: ItemTotals::default(),
                modules: Vec::new(),
                functions: Vec::new(),
            };
        }
    };

    let root_name = file_module_name(relative_path);
    let mut analyzer = Analyzer::new(relative_path.to_path_buf(), root_name);
    analyzer.visit_file(&file);
    analyzer.finish()
}

struct Analyzer {
    file_path: PathBuf,
    relative_path: PathBuf,
    attribute_count: u64,
    macro_invocations: u64,
    item_depths: Vec<u32>,
    totals: ItemTotals,
    /// Stack of in-progress module summaries. The bottom of the stack is
    /// the file's root module; nested inline `mod` blocks push a new frame.
    stack: Vec<ModuleSummary>,
    completed: Vec<ModuleSummary>,
    functions: Vec<FunctionSummary>,
}

impl Analyzer {
    fn new(relative_path: PathBuf, root_name: String) -> Self {
        let root = ModuleSummary {
            file_path: relative_path.clone(),
            module_path: root_name,
            depth: 0,
            totals: ItemTotals::default(),
        };
        Self {
            file_path: relative_path.clone(),
            relative_path,
            attribute_count: 0,
            macro_invocations: 0,
            item_depths: Vec::new(),
            totals: ItemTotals::default(),
            stack: vec![root],
            completed: Vec::new(),
            functions: Vec::new(),
        }
    }

    fn current(&mut self) -> &mut ModuleSummary {
        self.stack
            .last_mut()
            .expect("module stack is never empty: pushed on construction")
    }

    fn record_visibility(&mut self, vis: ItemVis) {
        match vis {
            ItemVis::Public => self.totals.pub_items += 1,
            ItemVis::PubCrate => self.totals.pub_crate_items += 1,
            ItemVis::Private => self.totals.private_items += 1,
            ItemVis::Restricted => {}
        }
    }

    fn record_function<T: quote::ToTokens>(
        &mut self,
        ident: &syn::Ident,
        body: &T,
        type_usage: FunctionTypeUsage,
    ) {
        let name = ident.to_string();
        let module_path = self.current().module_path.clone();
        let fq_path = format!("{module_path}::{name}");
        let node_hash = hash_tokens(body);
        self.functions.push(FunctionSummary {
            file_path: self.relative_path.clone(),
            module_path,
            name,
            fq_path,
            node_hash,
            type_usage,
        });
    }

    fn record_item(&mut self, kind: ItemKind) {
        let depth = self.current().depth;
        self.item_depths.push(depth);
        let cur = self.current();
        match kind {
            ItemKind::Fn => {
                cur.totals.functions += 1;
                self.totals.functions += 1;
            }
            ItemKind::Struct => {
                cur.totals.structs += 1;
                self.totals.structs += 1;
            }
            ItemKind::Enum => {
                cur.totals.enums += 1;
                self.totals.enums += 1;
            }
            ItemKind::Trait => {
                cur.totals.traits += 1;
                self.totals.traits += 1;
            }
            ItemKind::Impl => {
                cur.totals.impls += 1;
                self.totals.impls += 1;
            }
            ItemKind::Use => {
                cur.totals.uses += 1;
                self.totals.uses += 1;
            }
            ItemKind::Const => {
                cur.totals.consts += 1;
                self.totals.consts += 1;
            }
            ItemKind::Static => {
                cur.totals.statics += 1;
                self.totals.statics += 1;
            }
            ItemKind::TypeAlias => {
                cur.totals.type_aliases += 1;
                self.totals.type_aliases += 1;
            }
        }
    }

    fn finish(mut self) -> FileAnalysis {
        // Drain the stack — its bottom is the file's root module summary.
        while let Some(frame) = self.stack.pop() {
            self.completed.push(frame);
        }
        self.completed.sort_by(|a, b| a.module_path.cmp(&b.module_path));
        FileAnalysis {
            relative_path: self.relative_path,
            parse_error: None,
            attribute_count: self.attribute_count,
            macro_invocations: self.macro_invocations,
            item_depths: self.item_depths,
            totals: self.totals,
            modules: self.completed,
            functions: self.functions,
        }
    }
}

impl<'ast> Visit<'ast> for Analyzer {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if let Some(vis) = item_visibility(item) {
            self.record_visibility(vis);
        }
        match item {
            syn::Item::Fn(item_fn) => {
                self.record_item(ItemKind::Fn);
                let usage = collect_type_usage(&item_fn.sig, Some(&item_fn.block));
                self.record_function(&item_fn.sig.ident, item_fn, usage);
            }
            syn::Item::Struct(_) => self.record_item(ItemKind::Struct),
            syn::Item::Enum(_) => self.record_item(ItemKind::Enum),
            syn::Item::Trait(_) => self.record_item(ItemKind::Trait),
            syn::Item::Impl(item_impl) => {
                self.record_item(ItemKind::Impl);
                // `fn`s in `impl` blocks count toward both the impl's
                // enclosing module and the global function count.
                for assoc in &item_impl.items {
                    if let syn::ImplItem::Fn(impl_fn) = assoc {
                        let cur = self.current();
                        cur.totals.functions += 1;
                        self.totals.functions += 1;
                        let usage = collect_type_usage(&impl_fn.sig, Some(&impl_fn.block));
                        self.record_function(&impl_fn.sig.ident, impl_fn, usage);
                    }
                }
            }
            syn::Item::Use(_) => self.record_item(ItemKind::Use),
            syn::Item::Const(_) => self.record_item(ItemKind::Const),
            syn::Item::Static(_) => self.record_item(ItemKind::Static),
            syn::Item::Type(_) => self.record_item(ItemKind::TypeAlias),
            syn::Item::Mod(_) => { /* recurse below via visit::visit_item */ }
            _ => {}
        }
        visit::visit_item(self, item);
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        for attr in &item.attrs {
            self.visit_attribute(attr);
        }
        if let Some((_, items)) = &item.content {
            let parent_path = self.current().module_path.clone();
            let parent_depth = self.current().depth;
            let child = ModuleSummary {
                file_path: self.file_path.clone(),
                module_path: format!("{parent_path}::{}", item.ident),
                depth: parent_depth + 1,
                totals: ItemTotals::default(),
            };
            self.stack.push(child);
            for inner in items {
                self.visit_item(inner);
            }
            if let Some(frame) = self.stack.pop() {
                self.completed.push(frame);
            }
        }
        // `mod foo;` (external) is a declaration only; the referenced file is
        // walked separately when the inventory finds it.
    }

    fn visit_attribute(&mut self, _attr: &'ast syn::Attribute) {
        self.attribute_count += 1;
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.macro_invocations += 1;
        visit::visit_macro(self, mac);
    }
}

enum ItemKind {
    Fn,
    Struct,
    Enum,
    Trait,
    Impl,
    Use,
    Const,
    Static,
    TypeAlias,
}

pub(crate) enum ItemVis {
    Public,
    PubCrate,
    Restricted,
    Private,
}

fn item_visibility(item: &syn::Item) -> Option<ItemVis> {
    let vis = match item {
        syn::Item::Fn(i) => &i.vis,
        syn::Item::Struct(i) => &i.vis,
        syn::Item::Enum(i) => &i.vis,
        syn::Item::Trait(i) => &i.vis,
        syn::Item::Use(i) => &i.vis,
        syn::Item::Const(i) => &i.vis,
        syn::Item::Static(i) => &i.vis,
        syn::Item::Type(i) => &i.vis,
        syn::Item::Mod(i) => &i.vis,
        _ => return None,
    };
    Some(match vis {
        syn::Visibility::Public(_) => ItemVis::Public,
        syn::Visibility::Restricted(r) => {
            if r.path.is_ident("crate") {
                ItemVis::PubCrate
            } else {
                ItemVis::Restricted
            }
        }
        syn::Visibility::Inherited => ItemVis::Private,
    })
}

/// FNV-1a 64 over `t.to_token_stream().to_string()`. The token-stream
/// rendering is stable across reformatting (it serializes punctuation,
/// identifiers, and literals identically regardless of source-text
/// whitespace), so this hash satisfies the AstFuncId contract: stable
/// under formatting changes, but changes with any real edit.
fn hash_tokens<T: ToTokens>(t: &T) -> String {
    let rendered = t.to_token_stream().to_string();
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in rendered.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("h{:016x}", hash)
}

/// Collect every distinct type name reachable from a function's
/// signature and body. Names are canonicalized to their last-segment
/// form, so `std::vec::Vec` and a `use`-imported `Vec` collapse to a
/// single entry. The result feeds spec §2.4 level 16 (AST form).
pub(crate) fn collect_type_usage(
    sig: &syn::Signature,
    body: Option<&syn::Block>,
) -> FunctionTypeUsage {
    use std::collections::BTreeSet;
    let mut out = FunctionTypeUsage::default();

    let mut param_walker = TypeWalker::default();
    for input in &sig.inputs {
        if let syn::FnArg::Typed(pt) = input {
            param_walker.visit_type(&pt.ty);
        }
    }
    let TypeWalker { names: param_names, primitives: param_prims } = param_walker;
    out.param_distinct.extend(param_names.iter().cloned());
    out.distinct.extend(param_names);
    out.primitives.extend(param_prims);

    if let syn::ReturnType::Type(_, ty) = &sig.output {
        let mut ret_walker = TypeWalker::default();
        ret_walker.visit_type(ty);
        out.return_distinct.extend(ret_walker.names.iter().cloned());
        out.distinct.extend(ret_walker.names);
        out.primitives.extend(ret_walker.primitives);
    }

    if let Some(block) = body {
        let mut body_walker = TypeWalker::default();
        body_walker.visit_block(block);
        out.distinct.extend(body_walker.names);
        out.primitives.extend(body_walker.primitives);
    }

    let _ = BTreeSet::<String>::new; // silence unused-import lint
    out
}

#[derive(Default)]
struct TypeWalker {
    names: std::collections::BTreeSet<String>,
    primitives: std::collections::BTreeSet<String>,
}

impl<'ast> Visit<'ast> for TypeWalker {
    fn visit_type(&mut self, ty: &'ast syn::Type) {
        match ty {
            syn::Type::Path(tp) => {
                if let Some(last) = tp.path.segments.last() {
                    let name = last.ident.to_string();
                    if is_primitive_type_name(&name) {
                        self.primitives.insert(name.clone());
                    }
                    self.names.insert(name);
                }
            }
            syn::Type::Tuple(_) => { self.names.insert("()".into()); }
            syn::Type::Array(_) => { self.names.insert("[T;N]".into()); }
            syn::Type::Slice(_) => { self.names.insert("[T]".into()); }
            syn::Type::TraitObject(_) => { self.names.insert("dyn".into()); }
            syn::Type::ImplTrait(_) => { self.names.insert("impl".into()); }
            syn::Type::Ptr(_) => { self.names.insert("*const/*mut".into()); }
            syn::Type::BareFn(_) => { self.names.insert("fn()".into()); }
            _ => {}
        }
        visit::visit_type(self, ty);
    }
}

fn is_primitive_type_name(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "char"
            | "str"
            | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
            | "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
            | "f32" | "f64"
    )
}

fn file_module_name(path: &std::path::Path) -> String {
    // `src/lib.rs` and `src/main.rs` are the crate root; otherwise use the
    // file stem. Proper crate-qualified ModuleId resolution waits for cargo
    // metadata in Stage 5.
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
