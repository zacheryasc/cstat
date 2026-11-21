use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use syn::visit::Visit;
use syn::{File as SynFile, ImplItem, Item};

use crate::render;

/// Kind of root used when tracing call paths.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryPointKind {
    Main,
    Test,
    Bench,
}

impl EntryPointKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntryPointKind::Main => "main",
            EntryPointKind::Test => "test",
            EntryPointKind::Bench => "bench",
        }
    }
}

type StructFieldMap = HashMap<(String, String), String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionKind {
    Free,
    InherentMethod,
    TraitMethod,
    DropImpl,
}

#[derive(Debug, Clone)]
pub struct FunctionNode {
    pub kind: FunctionKind,
    pub self_type: Option<String>,
    pub trait_name: Option<String>,
    pub method_name: Option<String>,
    pub is_test_entry: bool,
    pub is_bench_entry: bool,
    pub is_test_support: bool,
}

/// Result of call graph analysis.
pub struct CallGraph {
    /// All known function names (qualified: Module::method or bare function).
    pub functions: Vec<String>,
    /// Function metadata keyed by canonical function name.
    pub function_nodes: HashMap<String, FunctionNode>,
    /// Edges: caller -> list of callees.
    pub adjacency: HashMap<String, Vec<String>>,
    /// Entry points (main, tests, and benchmarks).
    pub entry_points: Vec<String>,
    /// Entry point classification keyed by function name.
    pub entry_point_kinds: HashMap<String, EntryPointKind>,
}

#[derive(Debug, Clone)]
enum RawCallSite {
    Path {
        path: String,
        arg_functions: Vec<Option<String>>,
        arg_types: Vec<Option<String>>,
    },
    Method {
        receiver_name: Option<String>,
        receiver_hint: Option<String>,
        method: String,
    },
    LocalCallable {
        name: String,
    },
    ImplicitDrop {
        type_name: String,
    },
}

#[derive(Debug, Clone, Default)]
struct SignatureInfo {
    param_types: HashMap<String, String>,
    param_indices: HashMap<String, usize>,
    generic_params: HashSet<String>,
    return_type: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ItemContext {
    module_parts: Vec<String>,
    in_cfg_test: bool,
    in_tests_module: bool,
}

impl ItemContext {
    fn is_test_support_context(&self) -> bool {
        self.in_cfg_test || self.in_tests_module
    }

    fn child_module(&self, module: &syn::ItemMod) -> Self {
        let mut next = self.clone();
        next.in_cfg_test |= has_cfg_test_attr(&module.attrs);
        next.in_tests_module |= module.ident == "tests";
        next.module_parts.push(module.ident.to_string());
        next
    }
}

#[derive(Debug, Clone)]
struct FunctionRecord {
    name: String,
    node: FunctionNode,
    signature: SignatureInfo,
    entry_kind: Option<EntryPointKind>,
    raw_calls: Vec<RawCallSite>,
    callable_param_invocations: Vec<usize>,
}

/// AST visitor that extracts call sites from function bodies.
struct CallSiteExtractor {
    current_fn: String,
    module_parts: Vec<String>,
    is_test_support: bool,
    current_self_type: Option<String>,
    struct_fields: StructFieldMap,
    raw_calls: Vec<RawCallSite>,
    extra_records: Vec<FunctionRecord>,
    signature: SignatureInfo,
    callable_params: HashMap<String, usize>,
    callable_param_invocations: HashSet<usize>,
    local_types: HashMap<String, String>,
    closure_bindings: HashMap<String, String>,
    closure_counter: usize,
    callable_type_aliases: HashSet<String>,
}

impl CallSiteExtractor {
    fn new(
        current_fn: String,
        module_parts: Vec<String>,
        is_test_support: bool,
        current_self_type: Option<String>,
        struct_fields: &StructFieldMap,
        callable_type_aliases: &HashSet<String>,
        sig: Option<&syn::Signature>,
    ) -> Self {
        let mut signature = SignatureInfo::default();
        let mut callable_params = HashMap::new();
        signature.generic_params = sig.map(generic_type_param_names).unwrap_or_default();
        let callable_generics = sig.map(callable_generic_names).unwrap_or_default();
        signature.return_type =
            sig.and_then(|sig| return_type_hint(&sig.output, current_self_type.as_deref()));

        if let Some(sig) = sig {
            let mut typed_index = 0usize;
            for input in &sig.inputs {
                if let syn::FnArg::Typed(pat_type) = input {
                    if let Some(name) = pat_ident(&pat_type.pat) {
                        signature.param_indices.insert(name.clone(), typed_index);
                        if let Some(type_name) = type_hint_from_type(&pat_type.ty) {
                            signature.param_types.insert(name.clone(), type_name);
                        }
                        if is_callable_type(&pat_type.ty, &callable_generics, callable_type_aliases)
                        {
                            callable_params.insert(name.clone(), typed_index);
                        }
                    }
                    typed_index += 1;
                }
            }
        }

        let mut extractor = Self {
            current_fn,
            module_parts,
            is_test_support,
            current_self_type,
            struct_fields: struct_fields.clone(),
            raw_calls: Vec::new(),
            extra_records: Vec::new(),
            signature,
            callable_params,
            callable_param_invocations: HashSet::new(),
            local_types: HashMap::new(),
            closure_bindings: HashMap::new(),
            closure_counter: 0,
            callable_type_aliases: callable_type_aliases.clone(),
        };

        if let Some(sig) = sig {
            for input in &sig.inputs {
                match input {
                    syn::FnArg::Receiver(receiver) if receiver.reference.is_none() => {
                        if let Some(type_name) = extractor.current_self_type.clone() {
                            extractor.push_constructed_type(type_name);
                        }
                    }
                    syn::FnArg::Typed(pat_type) => {
                        if let Some(type_name) = owned_type_hint_from_type(&pat_type.ty) {
                            if !is_primitive_type_hint(&type_name) {
                                extractor.push_constructed_type(type_name);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        extractor
    }

    fn signature_info(&self) -> SignatureInfo {
        self.signature.clone()
    }

    fn finish(self) -> (Vec<RawCallSite>, Vec<usize>, Vec<FunctionRecord>) {
        let mut invoked: Vec<usize> = self.callable_param_invocations.into_iter().collect();
        invoked.sort_unstable();
        (self.raw_calls, invoked, self.extra_records)
    }

    fn push_constructed_type(&mut self, type_name: String) {
        if type_name != "?" {
            self.raw_calls.push(RawCallSite::ImplicitDrop { type_name });
        }
    }

    fn record_constructed_type(&mut self, type_name: String) {
        if !is_primitive_type_hint(&type_name) {
            self.push_constructed_type(type_name);
        }
    }

    fn record_constructed_expr(&mut self, expr: &syn::Expr) {
        if let Some(type_name) = expr_constructed_type(expr) {
            self.record_constructed_type(type_name);
        }
    }

    fn visit_macro_tokens(&mut self, tokens: proc_macro2::TokenStream) {
        use syn::parse::Parser;

        if let Ok(expr) = syn::parse2::<syn::Expr>(tokens.clone()) {
            self.visit_expr(&expr);
            return;
        }

        let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
        if let Ok(expressions) = parser.parse2(tokens) {
            for expr in expressions {
                self.visit_expr(&expr);
            }
        }
    }

    fn argument_function_and_type(&mut self, expr: &syn::Expr) -> (Option<String>, Option<String>) {
        let function = match expr {
            syn::Expr::Closure(closure) => Some(self.create_closure_record(closure)),
            _ => function_item_path_expr(expr),
        };
        let type_hint = expr_type_hint(
            expr,
            &self.local_types,
            &self.signature.param_types,
            self.current_self_type.as_deref(),
            &self.struct_fields,
        );
        (function, type_hint)
    }

    fn push_registered_callback(&mut self, callback: String) {
        self.raw_calls.push(RawCallSite::Path {
            path: callback,
            arg_functions: Vec::new(),
            arg_types: Vec::new(),
        });
    }

    fn create_closure_record(&mut self, closure: &syn::ExprClosure) -> String {
        self.closure_counter += 1;
        let closure_name = format!("{}::{{closure#{}}}", self.current_fn, self.closure_counter);
        let mut extractor = CallSiteExtractor::new(
            closure_name.clone(),
            self.module_parts.clone(),
            self.is_test_support,
            None,
            &self.struct_fields,
            &self.callable_type_aliases,
            None,
        );

        match closure.body.as_ref() {
            syn::Expr::Block(block) => extractor.visit_block(&block.block),
            expr => extractor.visit_expr(expr),
        }

        let signature = extractor.signature_info();
        let (raw_calls, callable_param_invocations, mut nested_extra) = extractor.finish();
        let node = FunctionNode {
            kind: FunctionKind::Free,
            self_type: None,
            trait_name: None,
            method_name: None,
            is_test_entry: false,
            is_bench_entry: false,
            is_test_support: self.is_test_support,
        };
        self.extra_records.push(FunctionRecord {
            name: closure_name.clone(),
            node,
            signature,
            entry_kind: None,
            raw_calls,
            callable_param_invocations,
        });
        self.extra_records.append(&mut nested_extra);
        closure_name
    }
}

impl<'ast> Visit<'ast> for CallSiteExtractor {
    fn visit_local(&mut self, local: &'ast syn::Local) {
        let local_name = pat_ident(&local.pat);
        let explicit_type = pat_type_hint(&local.pat);

        if let Some(init) = &local.init {
            if let syn::Expr::Closure(closure) = init.expr.as_ref() {
                if let Some(name) = local_name {
                    let closure_name = self.create_closure_record(closure);
                    self.closure_bindings.insert(name, closure_name);
                }
                return;
            }

            let init_type = expr_constructed_type(init.expr.as_ref());
            if let Some(name) = local_name {
                if let Some(type_name) = explicit_type.or_else(|| init_type.clone()) {
                    self.local_types.insert(name, type_name.clone());
                    self.record_constructed_type(type_name);
                }
            } else {
                self.record_constructed_expr(init.expr.as_ref());
            }

            self.visit_expr(init.expr.as_ref());
        }
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        let mut arg_functions = Vec::new();
        let mut arg_types = Vec::new();
        for arg in &node.args {
            let (arg_function, arg_type) = self.argument_function_and_type(arg);
            arg_functions.push(arg_function);
            arg_types.push(arg_type);
        }

        if let syn::Expr::Path(path) = node.func.as_ref() {
            if let Some(type_name) = constructed_drop_type_from_call_path(&path.path) {
                self.record_constructed_type(type_name);
            }
        }
        if let Some(path) = expr_to_callee_name(&node.func) {
            if let Some(closure_name) = self.closure_bindings.get(&path).cloned() {
                self.raw_calls.push(RawCallSite::Path {
                    path: closure_name,
                    arg_functions: Vec::new(),
                    arg_types: Vec::new(),
                });
            } else if let Some(index) = self.callable_params.get(&path).copied() {
                self.callable_param_invocations.insert(index);
                self.raw_calls
                    .push(RawCallSite::LocalCallable { name: path });
            } else {
                self.raw_calls.push(RawCallSite::Path {
                    path,
                    arg_functions,
                    arg_types,
                });
            }

            for arg in &node.args {
                self.visit_expr(arg);
            }
            return;
        }

        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let receiver_name = receiver_root_name(&node.receiver);
        let receiver_hint = receiver_type_hint(
            &node.receiver,
            &self.local_types,
            &self.signature.param_types,
            self.current_self_type.as_deref(),
            &self.struct_fields,
        );
        self.raw_calls.push(RawCallSite::Method {
            receiver_name,
            receiver_hint,
            method: node.method.to_string(),
        });
        self.visit_expr(&node.receiver);
        for arg in &node.args {
            let (arg_function, _) = self.argument_function_and_type(arg);
            if let Some(callback) = arg_function {
                self.push_registered_callback(callback);
            }
            self.visit_expr(arg);
        }
    }

    fn visit_expr_struct(&mut self, node: &'ast syn::ExprStruct) {
        self.record_constructed_expr(&syn::Expr::Struct(node.clone()));
        syn::visit::visit_expr_struct(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        self.record_constructed_expr(&syn::Expr::Path(node.clone()));
        syn::visit::visit_expr_path(self, node);
    }

    fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
        self.visit_macro_tokens(node.mac.tokens.clone());
    }

    fn visit_stmt_macro(&mut self, node: &'ast syn::StmtMacro) {
        self.visit_macro_tokens(node.mac.tokens.clone());
    }

    fn visit_expr_closure(&mut self, _node: &'ast syn::ExprClosure) {
        // Closure bodies are recorded under synthetic closure nodes when the closure
        // is bound or passed as a callable value. Do not attribute them to the
        // enclosing function.
    }
}

/// Try to extract a readable callee name from a call expression.
fn expr_to_callee_name(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Path(ep) => Some(expr_path_to_string(ep)),
        _ => None,
    }
}

fn expr_path_to_string(path: &syn::ExprPath) -> String {
    let segments: Vec<String> = path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    if let Some(qself) = &path.qself {
        if let Some(method) = segments.last() {
            let trait_name = if segments.len() > 1 {
                segments[..segments.len() - 1].join("::")
            } else {
                String::new()
            };
            let self_type = type_to_short_string(&qself.ty);
            if trait_name.is_empty() {
                format!("{self_type}::{method}")
            } else {
                format!("<{self_type} as {trait_name}>::{method}")
            }
        } else {
            type_to_short_string(&qself.ty)
        }
    } else {
        segments.join("::")
    }
}

/// Extract all function definitions and their call sites from a single file.
fn extract_calls_from_file(path: &Path, project_path: &Path) -> Vec<FunctionRecord> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let ast: SynFile = match syn::parse_file(&content) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let module = render::module_name_from_path(path, project_path);
    let base_modules: Vec<String> = if module == "main" || module == "lib" {
        Vec::new()
    } else {
        module.split("::").map(|s| s.to_string()).collect()
    };
    let context = ItemContext {
        module_parts: base_modules,
        in_cfg_test: false,
        in_tests_module: false,
    };
    let mut struct_fields = StructFieldMap::new();
    collect_struct_fields(&ast.items, &context, &mut struct_fields);
    let mut callable_type_aliases = HashSet::new();
    collect_callable_type_aliases(&ast.items, &context, &mut callable_type_aliases);
    let mut records = Vec::new();
    extract_items(
        &ast.items,
        path,
        project_path,
        &context,
        &struct_fields,
        &callable_type_aliases,
        &mut records,
    );
    records
}

fn collect_struct_fields(items: &[Item], context: &ItemContext, fields: &mut StructFieldMap) {
    for item in items {
        match item {
            Item::Struct(strukt) => {
                let type_name = qualify_name(&context.module_parts, &strukt.ident.to_string());
                if let syn::Fields::Named(named) = &strukt.fields {
                    for field in &named.named {
                        let Some(field_name) = field.ident.as_ref().map(|ident| ident.to_string())
                        else {
                            continue;
                        };
                        if let Some(field_type) = type_hint_from_type(&field.ty) {
                            fields.insert((type_name.clone(), field_name), field_type);
                        }
                    }
                }
            }
            Item::Mod(module) => {
                if let Some((_, nested_items)) = &module.content {
                    let next = context.child_module(module);
                    collect_struct_fields(nested_items, &next, fields);
                }
            }
            _ => {}
        }
    }
}

fn collect_callable_type_aliases(
    items: &[Item],
    context: &ItemContext,
    aliases: &mut HashSet<String>,
) {
    for item in items {
        match item {
            Item::Type(alias) if type_alias_target_is_callable(&alias.ty) => {
                let short = alias.ident.to_string();
                aliases.insert(short.clone());
                aliases.insert(qualify_name(&context.module_parts, &short));
            }
            Item::Mod(module) => {
                if let Some((_, nested_items)) = &module.content {
                    let next = context.child_module(module);
                    collect_callable_type_aliases(nested_items, &next, aliases);
                }
            }
            _ => {}
        }
    }
}

fn type_alias_target_is_callable(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::BareFn(_))
}

fn extract_items(
    items: &[Item],
    path: &Path,
    project_path: &Path,
    context: &ItemContext,
    struct_fields: &StructFieldMap,
    callable_type_aliases: &HashSet<String>,
    records: &mut Vec<FunctionRecord>,
) {
    for item in items {
        match item {
            Item::Fn(func) => {
                let name = func.sig.ident.to_string();
                let qualified = qualify_name(&context.module_parts, &name);
                let entry_kind = classify_entry_point(&name, &func.attrs, path, project_path);
                let is_test_support = context.is_test_support_context()
                    && !matches!(
                        entry_kind,
                        Some(EntryPointKind::Test) | Some(EntryPointKind::Bench)
                    );
                let mut extractor = CallSiteExtractor::new(
                    qualified.clone(),
                    context.module_parts.clone(),
                    is_test_support,
                    None,
                    struct_fields,
                    callable_type_aliases,
                    Some(&func.sig),
                );
                extractor.visit_block(&func.block);
                let signature = extractor.signature_info();
                let (raw_calls, callable_param_invocations, mut extra_records) = extractor.finish();
                let node = FunctionNode {
                    kind: FunctionKind::Free,
                    self_type: None,
                    trait_name: None,
                    method_name: None,
                    is_test_entry: matches!(entry_kind, Some(EntryPointKind::Test)),
                    is_bench_entry: matches!(entry_kind, Some(EntryPointKind::Bench)),
                    is_test_support,
                };
                records.push(FunctionRecord {
                    name: qualified,
                    node,
                    signature,
                    entry_kind,
                    raw_calls,
                    callable_param_invocations,
                });
                records.append(&mut extra_records);
            }
            Item::Impl(imp) => {
                let type_name =
                    qualify_name(&context.module_parts, &type_to_short_string(&imp.self_ty));
                let trait_name = imp.trait_.as_ref().map(|(_, path, _)| path_to_string(path));
                let kind = match trait_name.as_deref().map(last_path_segment) {
                    Some("Drop") => FunctionKind::DropImpl,
                    Some(_) => FunctionKind::TraitMethod,
                    None => FunctionKind::InherentMethod,
                };
                for impl_item in &imp.items {
                    if let ImplItem::Fn(method) = impl_item {
                        let method_name = method.sig.ident.to_string();
                        let qualified = format!("{}::{}", type_name, method_name);
                        let entry_kind =
                            classify_entry_point(&method_name, &method.attrs, path, project_path);
                        let is_test_support = context.is_test_support_context()
                            && !matches!(
                                entry_kind,
                                Some(EntryPointKind::Test) | Some(EntryPointKind::Bench)
                            );
                        let mut extractor = CallSiteExtractor::new(
                            qualified.clone(),
                            context.module_parts.clone(),
                            is_test_support,
                            Some(type_name.clone()),
                            struct_fields,
                            callable_type_aliases,
                            Some(&method.sig),
                        );
                        extractor.visit_block(&method.block);
                        let signature = extractor.signature_info();
                        let (raw_calls, callable_param_invocations, mut extra_records) =
                            extractor.finish();
                        let node = FunctionNode {
                            kind,
                            self_type: Some(type_name.clone()),
                            trait_name: trait_name.clone(),
                            method_name: Some(method_name),
                            is_test_entry: matches!(entry_kind, Some(EntryPointKind::Test)),
                            is_bench_entry: matches!(entry_kind, Some(EntryPointKind::Bench)),
                            is_test_support,
                        };
                        records.push(FunctionRecord {
                            name: qualified,
                            node,
                            signature,
                            entry_kind,
                            raw_calls,
                            callable_param_invocations,
                        });
                        records.append(&mut extra_records);
                    }
                }
            }
            Item::Mod(module) => {
                if let Some((_, nested_items)) = &module.content {
                    let next = context.child_module(module);
                    extract_items(
                        nested_items,
                        path,
                        project_path,
                        &next,
                        struct_fields,
                        callable_type_aliases,
                        records,
                    );
                }
            }
            _ => {}
        }
    }
}

fn qualify_name(module_parts: &[String], name: &str) -> String {
    if module_parts.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", module_parts.join("::"), name)
    }
}

fn classify_entry_point(
    name: &str,
    attrs: &[syn::Attribute],
    path: &Path,
    project_path: &Path,
) -> Option<EntryPointKind> {
    if has_attr_named(attrs, "test") {
        Some(EntryPointKind::Test)
    } else if has_attr_named(attrs, "bench") || is_under_dir(path, project_path, "benches") {
        Some(EntryPointKind::Bench)
    } else if name == "main" {
        Some(EntryPointKind::Main)
    } else {
        None
    }
}

fn has_attr_named(attrs: &[syn::Attribute], wanted: &str) -> bool {
    attrs.iter().any(|a| {
        a.path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == wanted)
    })
}

fn has_cfg_test_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "cfg")
            && match &attr.meta {
                syn::Meta::List(list) => list.tokens.to_string().contains("test"),
                _ => false,
            }
    })
}

fn is_under_dir(path: &Path, project_path: &Path, dir_name: &str) -> bool {
    let rel = path.strip_prefix(project_path).unwrap_or(path);
    rel.components()
        .any(|component| component.as_os_str() == dir_name)
}

fn path_to_string(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn last_path_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}

fn short_type_name(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}

/// Convert a Type to a short string.
fn type_to_short_string(ty: &syn::Type) -> String {
    type_hint_from_type(ty).unwrap_or_else(|| "?".to_string())
}

fn type_hint_from_type(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Reference(reference) => type_hint_from_type(&reference.elem),
        syn::Type::Path(tp) => {
            let last = tp.path.segments.last()?;
            if last.ident == "Box" {
                if let syn::PathArguments::AngleBracketed(args) = &last.arguments {
                    for arg in &args.args {
                        if let syn::GenericArgument::Type(inner) = arg {
                            return type_hint_from_type(inner);
                        }
                    }
                }
            }
            Some(path_to_string(&tp.path))
        }
        _ => None,
    }
}

fn return_type_hint(output: &syn::ReturnType, current_self_type: Option<&str>) -> Option<String> {
    match output {
        syn::ReturnType::Default => None,
        syn::ReturnType::Type(_, ty) => owned_return_type_hint_from_type(ty, current_self_type),
    }
}

fn owned_return_type_hint_from_type(
    ty: &syn::Type,
    current_self_type: Option<&str>,
) -> Option<String> {
    match ty {
        syn::Type::Reference(_) => None,
        syn::Type::Paren(paren) => owned_return_type_hint_from_type(&paren.elem, current_self_type),
        syn::Type::Path(tp) => {
            let last = tp.path.segments.last()?;
            if last.ident == "Self" {
                return current_self_type.map(str::to_string);
            }
            if matches!(last.ident.to_string().as_str(), "Option" | "Result") {
                if let syn::PathArguments::AngleBracketed(args) = &last.arguments {
                    for arg in &args.args {
                        if let syn::GenericArgument::Type(inner) = arg {
                            return owned_return_type_hint_from_type(inner, current_self_type);
                        }
                    }
                }
            }
            let name = path_to_string(&tp.path);
            if name == "Self" {
                current_self_type.map(str::to_string)
            } else {
                Some(name)
            }
        }
        _ => None,
    }
}

fn pat_ident(pat: &syn::Pat) -> Option<String> {
    match pat {
        syn::Pat::Ident(ident) => Some(ident.ident.to_string()),
        syn::Pat::Type(pat_type) => pat_ident(&pat_type.pat),
        syn::Pat::Reference(reference) => pat_ident(&reference.pat),
        _ => None,
    }
}

fn pat_type_hint(pat: &syn::Pat) -> Option<String> {
    match pat {
        syn::Pat::Type(pat_type) => type_hint_from_type(&pat_type.ty),
        syn::Pat::Reference(reference) => pat_type_hint(&reference.pat),
        _ => None,
    }
}

fn function_item_path_expr(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Path(path) if path.qself.is_none() => Some(path_to_string(&path.path)),
        syn::Expr::Reference(reference) => function_item_path_expr(&reference.expr),
        syn::Expr::Cast(cast) => function_item_path_expr(&cast.expr),
        syn::Expr::Paren(paren) => function_item_path_expr(&paren.expr),
        syn::Expr::Group(group) => function_item_path_expr(&group.expr),
        _ => None,
    }
}

fn expr_constructed_type(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Struct(strukt) => Some(path_to_string(&strukt.path)),
        syn::Expr::Path(path) if path.qself.is_none() => {
            let name = path_to_string(&path.path);
            if first_type_segment(&name).is_some() {
                Some(name)
            } else {
                None
            }
        }
        syn::Expr::Call(call) => match call.func.as_ref() {
            syn::Expr::Path(path) => constructed_type_from_call_path(&path.path),
            _ => None,
        },
        syn::Expr::Reference(reference) => expr_constructed_type(&reference.expr),
        syn::Expr::Paren(paren) => expr_constructed_type(&paren.expr),
        syn::Expr::Group(group) => expr_constructed_type(&group.expr),
        _ => None,
    }
}

fn constructed_type_from_call_path(path: &syn::Path) -> Option<String> {
    constructed_drop_type_from_call_path(path)
}

fn constructed_drop_type_from_call_path(path: &syn::Path) -> Option<String> {
    let mut segments: Vec<String> = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    if segments.is_empty() {
        return None;
    }
    if segments.len() == 1 {
        let name = segments.pop().unwrap();
        return if first_type_segment(&name).is_some() {
            Some(name)
        } else {
            None
        };
    }
    if segments.last().is_some_and(|last| last == "new") {
        segments.pop();
        let receiver = segments.join("::");
        return if first_type_segment(&receiver).is_some() {
            Some(receiver)
        } else {
            None
        };
    }
    None
}

fn first_type_segment(path: &str) -> Option<&str> {
    path.split("::")
        .find(|segment| segment.chars().next().is_some_and(|c| c.is_uppercase()))
}

fn receiver_root_name(receiver: &syn::Expr) -> Option<String> {
    match receiver {
        syn::Expr::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => {
            Some(path.path.segments[0].ident.to_string())
        }
        syn::Expr::Reference(reference) => receiver_root_name(&reference.expr),
        syn::Expr::Field(field) => receiver_root_name(&field.base),
        syn::Expr::Paren(paren) => receiver_root_name(&paren.expr),
        syn::Expr::Group(group) => receiver_root_name(&group.expr),
        _ => None,
    }
}

fn expr_type_hint(
    expr: &syn::Expr,
    local_types: &HashMap<String, String>,
    param_types: &HashMap<String, String>,
    current_self_type: Option<&str>,
    struct_fields: &StructFieldMap,
) -> Option<String> {
    match expr {
        syn::Expr::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => {
            let name = path.path.segments[0].ident.to_string();
            if name == "self" {
                return current_self_type.map(str::to_string);
            }
            local_types
                .get(&name)
                .or_else(|| param_types.get(&name))
                .cloned()
                .or_else(|| expr_constructed_type(expr))
        }
        syn::Expr::Reference(reference) => expr_type_hint(
            &reference.expr,
            local_types,
            param_types,
            current_self_type,
            struct_fields,
        ),
        syn::Expr::Field(field) => {
            let base_type = expr_type_hint(
                &field.base,
                local_types,
                param_types,
                current_self_type,
                struct_fields,
            )?;
            let field_name = match &field.member {
                syn::Member::Named(ident) => ident.to_string(),
                syn::Member::Unnamed(_) => return None,
            };
            lookup_field_type(&base_type, &field_name, struct_fields)
        }
        syn::Expr::Struct(_) | syn::Expr::Call(_) => expr_constructed_type(expr),
        syn::Expr::Paren(paren) => expr_type_hint(
            &paren.expr,
            local_types,
            param_types,
            current_self_type,
            struct_fields,
        ),
        syn::Expr::Group(group) => expr_type_hint(
            &group.expr,
            local_types,
            param_types,
            current_self_type,
            struct_fields,
        ),
        _ => None,
    }
}

fn lookup_field_type(
    base_type: &str,
    field_name: &str,
    struct_fields: &StructFieldMap,
) -> Option<String> {
    struct_fields
        .get(&(base_type.to_string(), field_name.to_string()))
        .cloned()
        .or_else(|| {
            let short_base = short_type_name(base_type);
            struct_fields.iter().find_map(|((ty, field), value)| {
                (field == field_name && short_type_name(ty) == short_base).then(|| value.clone())
            })
        })
}

fn receiver_type_hint(
    receiver: &syn::Expr,
    local_types: &HashMap<String, String>,
    param_types: &HashMap<String, String>,
    current_self_type: Option<&str>,
    struct_fields: &StructFieldMap,
) -> Option<String> {
    expr_type_hint(
        receiver,
        local_types,
        param_types,
        current_self_type,
        struct_fields,
    )
}

fn generic_type_param_names(sig: &syn::Signature) -> HashSet<String> {
    sig.generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(ty) => Some(ty.ident.to_string()),
            _ => None,
        })
        .collect()
}

fn owned_type_hint_from_type(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Reference(_) => None,
        _ => type_hint_from_type(ty),
    }
}

fn is_primitive_type_hint(type_name: &str) -> bool {
    matches!(
        type_name,
        "bool"
            | "char"
            | "str"
            | "String"
            | "usize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "isize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "f32"
            | "f64"
    )
}

fn callable_generic_names(sig: &syn::Signature) -> HashSet<String> {
    let mut names = HashSet::new();
    for param in &sig.generics.params {
        if let syn::GenericParam::Type(ty) = param {
            if ty.bounds.iter().any(|bound| trait_bound_is_fn(bound)) {
                names.insert(ty.ident.to_string());
            }
        }
    }
    if let Some(where_clause) = &sig.generics.where_clause {
        for predicate in &where_clause.predicates {
            if let syn::WherePredicate::Type(predicate) = predicate {
                if predicate
                    .bounds
                    .iter()
                    .any(|bound| trait_bound_is_fn(bound))
                {
                    if let syn::Type::Path(path) = &predicate.bounded_ty {
                        if path.path.segments.len() == 1 {
                            names.insert(path.path.segments[0].ident.to_string());
                        }
                    }
                }
            }
        }
    }
    names
}

fn trait_bound_is_fn(bound: &syn::TypeParamBound) -> bool {
    match bound {
        syn::TypeParamBound::Trait(trait_bound) => {
            trait_bound.path.segments.last().is_some_and(|segment| {
                let ident = segment.ident.to_string();
                ident == "Fn" || ident == "FnMut" || ident == "FnOnce"
            })
        }
        _ => false,
    }
}

fn is_callable_type(
    ty: &syn::Type,
    callable_generics: &HashSet<String>,
    callable_type_aliases: &HashSet<String>,
) -> bool {
    match ty {
        syn::Type::BareFn(_) => true,
        syn::Type::ImplTrait(impl_trait) => impl_trait.bounds.iter().any(trait_bound_is_fn),
        syn::Type::Path(path) => {
            let name = path_to_string(&path.path);
            let short = last_path_segment(&name);
            callable_type_aliases.contains(&name)
                || callable_type_aliases.contains(short)
                || (path.path.segments.len() == 1 && callable_generics.contains(short))
        }
        _ => false,
    }
}

/// Extern-crate aliases for the package being analyzed.
///
/// In a standard lib+bin Cargo package, the binary's `main` reaches its own
/// library through the package's crate name (e.g. `mvp_system::run_from_args`).
/// Those call paths are semantically identical to `crate::` from inside the
/// library, so the resolver strips the leading `<alias>::` just like `crate::`.
/// Covers both `[package].name` and a custom `[lib].name`, with `-` -> `_`.
fn crate_aliases(project_path: &Path) -> Vec<String> {
    let manifest = match std::fs::read_to_string(project_path.join("Cargo.toml")) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };
    let mut names = Vec::new();
    let mut section = "";
    for line in manifest.lines() {
        let trimmed = line.trim();
        if let Some(header) = trimmed.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            section = header.trim();
            continue;
        }
        if !matches!(section, "package" | "lib") {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            if key.trim() == "name" {
                let name = value.trim().trim_matches('"');
                if !name.is_empty() {
                    names.push(name.replace('-', "_"));
                }
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

struct Resolver<'a> {
    known: HashMap<String, usize>,
    nodes: &'a HashMap<String, FunctionNode>,
    trait_aliases: HashMap<String, String>,
    full_trait_aliases: HashMap<String, String>,
    methods_by_name: HashMap<String, Vec<String>>,
    methods_by_type_and_name: HashMap<(String, String), Vec<String>>,
    crate_aliases: Vec<String>,
}

impl<'a> Resolver<'a> {
    fn new(
        functions: &[String],
        nodes: &'a HashMap<String, FunctionNode>,
        crate_aliases: Vec<String>,
    ) -> Self {
        let known: HashMap<String, usize> = functions
            .iter()
            .enumerate()
            .map(|(index, name)| (name.clone(), index))
            .collect();
        let mut trait_alias_buckets: HashMap<String, Vec<String>> = HashMap::new();
        let mut full_trait_aliases = HashMap::new();
        let mut methods_by_name: HashMap<String, Vec<String>> = HashMap::new();
        let mut methods_by_type_and_name: HashMap<(String, String), Vec<String>> = HashMap::new();

        for (name, node) in nodes {
            if let Some(method) = &node.method_name {
                methods_by_name
                    .entry(method.clone())
                    .or_default()
                    .push(name.clone());
                if let Some(self_type) = &node.self_type {
                    methods_by_type_and_name
                        .entry((self_type.clone(), method.clone()))
                        .or_default()
                        .push(name.clone());
                    let short_self = short_type_name(self_type).to_string();
                    methods_by_type_and_name
                        .entry((short_self.clone(), method.clone()))
                        .or_default()
                        .push(name.clone());
                    if let Some(trait_name) = &node.trait_name {
                        for trait_alias in [
                            trait_name.clone(),
                            last_path_segment(trait_name).to_string(),
                        ] {
                            trait_alias_buckets
                                .entry(format!("{trait_alias}::{method}"))
                                .or_default()
                                .push(name.clone());
                            full_trait_aliases.insert(
                                format!("<{self_type} as {trait_alias}>::{method}"),
                                name.clone(),
                            );
                            full_trait_aliases.insert(
                                format!("<{short_self} as {trait_alias}>::{method}"),
                                name.clone(),
                            );
                        }
                    }
                }
            }
        }

        let trait_aliases = trait_alias_buckets
            .into_iter()
            .filter_map(|(alias, targets)| {
                if targets.len() == 1 {
                    Some((alias, targets[0].clone()))
                } else {
                    None
                }
            })
            .collect();

        Self {
            known,
            nodes,
            trait_aliases,
            full_trait_aliases,
            methods_by_name,
            methods_by_type_and_name,
            crate_aliases,
        }
    }

    /// Resolve a callee name against known function names.
    /// Tries normalized path prefixes first, then exact match, caller-local sibling match,
    /// trait aliases, and suffix match.
    fn resolve_path(&self, callee: &str, caller: &str) -> Option<String> {
        let candidates = self.normalized_path_candidates(callee, caller);

        for candidate in &candidates {
            if self.known.contains_key(candidate) {
                return Some(candidate.clone());
            }
        }

        for candidate in &candidates {
            if !candidate.contains("::") {
                let caller_module = self.caller_module_parts(caller);
                if !caller_module.is_empty() {
                    let sibling = qualify_name(&caller_module, candidate);
                    if self.known.contains_key(&sibling) {
                        return Some(sibling);
                    }
                }
            }
        }

        for candidate in &candidates {
            if let Some(resolved) = self.full_trait_aliases.get(candidate) {
                return Some(resolved.clone());
            }
            if let Some(resolved) = self.trait_aliases.get(candidate) {
                return Some(resolved.clone());
            }
        }

        for candidate in &candidates {
            let suffix = format!("::{}", candidate);
            let matches: Vec<&String> =
                self.known.keys().filter(|k| k.ends_with(&suffix)).collect();
            if matches.len() == 1 {
                return Some(matches[0].clone());
            }

            if candidate.contains("::") {
                let matches: Vec<&String> = self
                    .known
                    .keys()
                    .filter(|k| k.ends_with(candidate))
                    .collect();
                if matches.len() == 1 {
                    return Some(matches[0].clone());
                }
            }
        }
        None
    }

    fn normalized_path_candidates(&self, callee: &str, caller: &str) -> Vec<String> {
        let mut candidates = Vec::new();
        for alias in &self.crate_aliases {
            let prefix = format!("{alias}::");
            if let Some(rest) = callee.strip_prefix(prefix.as_str()) {
                candidates.push(rest.to_string());
            }
        }
        if let Some(rest) = callee.strip_prefix("crate::") {
            candidates.push(rest.to_string());
        } else if let Some(rest) = callee.strip_prefix("self::") {
            candidates.push(qualify_name(&self.caller_module_parts(caller), rest));
        } else if let Some(rest) = callee.strip_prefix("super::") {
            let mut module_parts = self.caller_module_parts(caller);
            let mut remaining = rest;
            while let Some(next) = remaining.strip_prefix("super::") {
                if !module_parts.is_empty() {
                    module_parts.pop();
                }
                remaining = next;
            }
            if !module_parts.is_empty() {
                module_parts.pop();
            }
            candidates.push(qualify_name(&module_parts, remaining));
        } else if let Some(rest) = callee.strip_prefix("Self::") {
            if let Some(self_type) = self
                .nodes
                .get(caller)
                .and_then(|node| node.self_type.as_ref())
            {
                candidates.push(format!("{self_type}::{rest}"));
            }
        }
        candidates.push(callee.to_string());
        let mut seen = HashSet::new();
        candidates.retain(|candidate| seen.insert(candidate.clone()));
        candidates
    }

    fn caller_module_parts(&self, caller: &str) -> Vec<String> {
        let base_caller = caller.split("::{closure#").next().unwrap_or(caller);
        if let Some(self_type) = self
            .nodes
            .get(base_caller)
            .and_then(|node| node.self_type.as_ref())
        {
            let mut parts: Vec<String> = self_type.split("::").map(str::to_string).collect();
            parts.pop();
            return parts;
        }
        base_caller
            .rsplit_once("::")
            .map(|(parent, _)| parent.split("::").map(str::to_string).collect())
            .unwrap_or_default()
    }

    fn resolve_method(
        &self,
        receiver_hint: Option<&str>,
        method: &str,
        caller: &str,
    ) -> Option<String> {
        if let Some(receiver_hint) = receiver_hint {
            let mut type_candidates = Vec::new();
            if !receiver_hint.contains("::") {
                let caller_module = self.caller_module_parts(caller);
                if !caller_module.is_empty() {
                    type_candidates.push(qualify_name(&caller_module, receiver_hint));
                }
            }
            type_candidates.push(receiver_hint.to_string());
            for node in self.nodes.values() {
                if let Some(self_type) = &node.self_type {
                    if self_type == receiver_hint || short_type_name(self_type) == receiver_hint {
                        type_candidates.push(self_type.clone());
                    }
                }
            }
            let mut seen_types = HashSet::new();
            type_candidates.retain(|type_name| seen_types.insert(type_name.clone()));

            let method_key = method.to_string();
            for type_name in &type_candidates {
                if let Some(methods) = self
                    .methods_by_type_and_name
                    .get(&(type_name.clone(), method_key.clone()))
                {
                    if let Some(chosen) = choose_method_candidate(methods.clone(), self.nodes) {
                        return Some(chosen);
                    }
                }
            }

            let mut candidates = Vec::new();
            for type_name in type_candidates {
                if let Some(methods) = self
                    .methods_by_type_and_name
                    .get(&(type_name, method_key.clone()))
                {
                    candidates.extend(methods.iter().cloned());
                }
            }
            candidates.sort();
            candidates.dedup();
            return choose_method_candidate(candidates, self.nodes);
        }

        self.methods_by_name
            .get(method)
            .and_then(|candidates| choose_method_candidate(candidates.clone(), self.nodes))
    }
}

fn choose_method_candidate(
    candidates: Vec<String>,
    nodes: &HashMap<String, FunctionNode>,
) -> Option<String> {
    if candidates.len() == 1 {
        return candidates.into_iter().next();
    }
    let inherent: Vec<String> = candidates
        .iter()
        .filter(|name| {
            nodes
                .get(*name)
                .is_some_and(|node| node.kind == FunctionKind::InherentMethod)
        })
        .cloned()
        .collect();
    if inherent.len() == 1 {
        inherent.into_iter().next()
    } else {
        None
    }
}

fn concrete_generic_method_edges(
    callee_record: &FunctionRecord,
    arg_types: &[Option<String>],
    resolver: &Resolver<'_>,
    caller: &str,
) -> Vec<String> {
    if callee_record.signature.generic_params.is_empty() {
        return Vec::new();
    }

    let mut generic_bindings: HashMap<String, String> = HashMap::new();
    for (param_name, param_type) in &callee_record.signature.param_types {
        if !callee_record.signature.generic_params.contains(param_type) {
            continue;
        }
        let Some(index) = callee_record.signature.param_indices.get(param_name) else {
            continue;
        };
        if let Some(Some(arg_type)) = arg_types.get(*index) {
            generic_bindings.insert(param_type.clone(), arg_type.clone());
        }
    }

    let mut out = Vec::new();
    for raw in &callee_record.raw_calls {
        let RawCallSite::Method {
            receiver_name,
            receiver_hint,
            method,
        } = raw
        else {
            continue;
        };

        let concrete = receiver_name
            .as_ref()
            .and_then(|name| callee_record.signature.param_indices.get(name))
            .and_then(|index| arg_types.get(*index))
            .and_then(|arg_type| arg_type.clone())
            .or_else(|| {
                receiver_hint
                    .as_ref()
                    .and_then(|hint| generic_bindings.get(hint).cloned())
            });
        let Some(concrete) = concrete else {
            continue;
        };
        if let Some(resolved) = resolver.resolve_method(Some(&concrete), method, caller) {
            out.push(resolved);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Build the call graph from all project files.
pub fn build_call_graph(rs_files: &[PathBuf], project_path: &Path) -> CallGraph {
    let mut records = Vec::new();

    for file in rs_files {
        records.extend(extract_calls_from_file(file, project_path));
    }

    let functions: Vec<String> = records.iter().map(|record| record.name.clone()).collect();
    let function_nodes: HashMap<String, FunctionNode> = records
        .iter()
        .map(|record| (record.name.clone(), record.node.clone()))
        .collect();
    let entry_point_kinds: HashMap<String, EntryPointKind> = records
        .iter()
        .filter_map(|record| {
            record
                .entry_kind
                .clone()
                .map(|kind| (record.name.clone(), kind))
        })
        .collect();
    let entry_points: Vec<String> = records
        .iter()
        .filter(|record| entry_point_kinds.contains_key(&record.name))
        .map(|record| record.name.clone())
        .collect();

    let resolver = Resolver::new(&functions, &function_nodes, crate_aliases(project_path));
    let callable_summaries: HashMap<String, Vec<usize>> = records
        .iter()
        .filter(|record| !record.callable_param_invocations.is_empty())
        .map(|record| {
            (
                record.name.clone(),
                record.callable_param_invocations.clone(),
            )
        })
        .collect();
    let records_by_name: HashMap<String, &FunctionRecord> = records
        .iter()
        .map(|record| (record.name.clone(), record))
        .collect();

    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    let mut push_edge = |caller: &str, callee: String| {
        if callee != caller {
            adjacency
                .entry(caller.to_string())
                .or_default()
                .push(callee);
        }
    };

    for record in &records {
        for raw in &record.raw_calls {
            match raw {
                RawCallSite::Path {
                    path,
                    arg_functions,
                    arg_types,
                } => {
                    if let Some(resolved) = resolver.resolve_path(path, &record.name) {
                        push_edge(&record.name, resolved.clone());
                        if let Some(invoked_positions) = callable_summaries.get(&resolved) {
                            for position in invoked_positions {
                                if let Some(Some(arg_path)) = arg_functions.get(*position) {
                                    if let Some(arg_function) =
                                        resolver.resolve_path(arg_path, &record.name)
                                    {
                                        push_edge(&resolved, arg_function);
                                    }
                                }
                            }
                        }
                        if let Some(callee_record) = records_by_name.get(&resolved) {
                            for callee in concrete_generic_method_edges(
                                callee_record,
                                arg_types,
                                &resolver,
                                &resolved,
                            ) {
                                push_edge(&resolved, callee);
                            }
                            if let Some(return_type) = &callee_record.signature.return_type {
                                if let Some(drop_impl) =
                                    resolver.resolve_method(Some(return_type), "drop", &record.name)
                                {
                                    if function_nodes
                                        .get(&drop_impl)
                                        .is_some_and(|node| node.kind == FunctionKind::DropImpl)
                                    {
                                        push_edge(&record.name, drop_impl);
                                    }
                                }
                            }
                        }
                    } else {
                        for arg_path in arg_functions.iter().flatten() {
                            if let Some(arg_function) =
                                resolver.resolve_path(arg_path, &record.name)
                            {
                                push_edge(&record.name, arg_function);
                            }
                        }
                    }
                }
                RawCallSite::Method {
                    receiver_name: _,
                    receiver_hint,
                    method,
                } => {
                    if let Some(resolved) =
                        resolver.resolve_method(receiver_hint.as_deref(), method, &record.name)
                    {
                        push_edge(&record.name, resolved);
                    }
                }
                RawCallSite::LocalCallable { name } => {
                    let _ = name;
                }
                RawCallSite::ImplicitDrop { type_name } => {
                    if let Some(resolved) =
                        resolver.resolve_method(Some(type_name), "drop", &record.name)
                    {
                        if function_nodes
                            .get(&resolved)
                            .is_some_and(|node| node.kind == FunctionKind::DropImpl)
                        {
                            push_edge(&record.name, resolved);
                        }
                    }
                }
            }
        }
    }

    // Deduplicate adjacency lists while preserving order.
    for callees in adjacency.values_mut() {
        let mut seen = HashSet::new();
        callees.retain(|c| seen.insert(c.clone()));
    }

    CallGraph {
        functions,
        function_nodes,
        adjacency,
        entry_points,
        entry_point_kinds,
    }
}

/// Return entry points matching a specific kind.
pub fn entry_points_by_kind(call_graph: &CallGraph, kind: EntryPointKind) -> Vec<String> {
    call_graph
        .entry_points
        .iter()
        .filter(|entry| call_graph.entry_point_kinds.get(*entry) == Some(&kind))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("cstat-flow-{name}-{unique}"));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        root
    }

    fn assert_edge(graph: &CallGraph, caller: &str, callee: &str) {
        assert!(
            graph
                .adjacency
                .get(caller)
                .is_some_and(|callees| callees.contains(&callee.to_string())),
            "{caller} should call {callee}: {:?}",
            graph.adjacency.get(caller)
        );
    }

    #[test]
    fn flow_mvp_chat_normalizes_qualified_direct_calls() {
        let root = temp_project("mvp-qualified");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
fn root_helper() {}
fn outer(_value: ()) {}
fn inner() {}
mod nested {
    pub fn target() {}
    pub mod child {
        pub fn local_helper() {}
        pub fn exercise() {
            super::target();
            crate::root_helper();
            self::local_helper();
            outer(crate::inner());
        }
    }
}
struct RuntimeConfigProfile;
impl RuntimeConfigProfile {
    fn default_provider() {}
    fn exercise() { Self::default_provider(); }
}
fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        assert_edge(&graph, "nested::child::exercise", "nested::target");
        assert_edge(
            &graph,
            "nested::child::exercise",
            "nested::child::local_helper",
        );
        assert_edge(&graph, "nested::child::exercise", "root_helper");
        assert_edge(&graph, "nested::child::exercise", "outer");
        assert_edge(&graph, "nested::child::exercise", "inner");
        assert_edge(
            &graph,
            "RuntimeConfigProfile::exercise",
            "RuntimeConfigProfile::default_provider",
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flow_mvp_chat_resolves_crate_qualified_entry_shim() {
        // A lib+bin package's binary reaches its own library through the
        // package's crate name (e.g. `fixture::run_from_args`). That call path
        // is semantically `crate::` and must resolve to the crate-root symbol;
        // otherwise the entire production call tree reads as dead code.
        let root = temp_project("mvp-shim");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
fn run_from_args() { leaf(); }
fn leaf() {}
fn main() { fixture::run_from_args(); }
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        assert_edge(&graph, "main", "run_from_args");
        assert_edge(&graph, "run_from_args", "leaf");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flow_mvp_chat_tracks_wrapper_function_item_seam() {
        let root = temp_project("mvp-seam");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
fn ensure_orch_binary() -> bool {
    ensure_orch_binary_with("config", "root", "default-orch", run_status)
}
fn ensure_orch_binary_with(
    _config: &str,
    _root: &str,
    _default_orch: &str,
    run_status_fn: fn() -> bool,
) -> bool {
    run_status_fn()
}
fn run_status() -> bool { true }
fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        assert_edge(&graph, "ensure_orch_binary", "ensure_orch_binary_with");
        assert_edge(&graph, "ensure_orch_binary_with", "run_status");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flow_mvp_chat_tracks_function_pointer_type_alias_seam() {
        let root = temp_project("mvp-alias-seam");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
type RunStatusFn = fn(&str, &[&str], &str) -> bool;
fn ensure_orch_binary() -> bool {
    ensure_orch_binary_with(run_status)
}
fn ensure_orch_binary_with(run_status_fn: RunStatusFn) -> bool {
    run_status_fn("cargo", &["build"], "build orchestrator")
}
fn run_status(_program: &str, _args: &[&str], _label: &str) -> bool { true }
fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        assert_edge(&graph, "ensure_orch_binary", "ensure_orch_binary_with");
        assert_edge(&graph, "ensure_orch_binary_with", "run_status");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flow_mvp_chat_walks_vec_macro_and_array_method_chains() {
        let root = temp_project("mvp-vec-macro");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
struct RuntimeConfigProfile;
struct RelayMode;
struct Config {
    config_profile: RuntimeConfigProfile,
    relay_mode: RelayMode,
}
impl RuntimeConfigProfile {
    fn as_str(self) -> &'static str { "local" }
}
impl RelayMode {
    fn as_str(&self) -> &'static str { "default" }
}
fn relay_mode_env_value(_mode: &RelayMode) -> &'static str { "default" }
impl Config {
    fn orchestrator_cli_args(&self) {
        let mut args = vec![
            "--runtime-config".to_owned(),
            self.config_profile.as_str().to_owned(),
            self.relay_mode.as_str().to_owned(),
        ];
        args.extend([
            "--relay-mode".to_owned(),
            relay_mode_env_value(&self.relay_mode).to_owned(),
        ]);
    }
}
fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        assert_edge(
            &graph,
            "Config::orchestrator_cli_args",
            "RuntimeConfigProfile::as_str",
        );
        assert_edge(&graph, "Config::orchestrator_cli_args", "RelayMode::as_str");
        assert_edge(
            &graph,
            "Config::orchestrator_cli_args",
            "relay_mode_env_value",
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flow_mvp_chat_prefers_receiver_type_in_caller_module_when_short_names_collide() {
        let root = temp_project("mvp-short-collision");
        let bin_dir = root.join("src/bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let main = bin_dir.join("mvp_chat.rs");
        fs::write(
            &main,
            r#"
mod other {
    pub struct RuntimeConfigProfile;
    impl RuntimeConfigProfile {
        pub fn as_str(&self) -> &'static str { "other" }
    }
    pub fn relay_mode_env_value() {}
}
struct RuntimeConfigProfile;
struct Config { config_profile: RuntimeConfigProfile }
impl RuntimeConfigProfile {
    fn as_str(&self) -> &'static str { "local" }
}
fn relay_mode_env_value() {}
impl Config {
    fn orchestrator_cli_args(&self) {
        self.config_profile.as_str();
        relay_mode_env_value();
    }
}
fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        assert_edge(
            &graph,
            "bin::mvp_chat::Config::orchestrator_cli_args",
            "bin::mvp_chat::RuntimeConfigProfile::as_str",
        );
        assert_edge(
            &graph,
            "bin::mvp_chat::Config::orchestrator_cli_args",
            "bin::mvp_chat::relay_mode_env_value",
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flow_mvp_chat_adds_drop_edges_for_self_returning_associated_results() {
        let root = temp_project("mvp-self-return-drop");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
struct OrchChild;
impl OrchChild {
    fn spawn() -> Result<Self, String> { Ok(OrchChild) }
    fn shutdown(&mut self, _wait: bool) {}
}
impl Drop for OrchChild {
    fn drop(&mut self) { self.shutdown(false); }
}
fn exercise() {
    let _child = OrchChild::spawn().expect("spawn");
}
fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        assert_edge(&graph, "exercise", "OrchChild::spawn");
        assert_edge(&graph, "exercise", "OrchChild::drop");
        assert_edge(&graph, "OrchChild::drop", "OrchChild::shutdown");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flow_mvp_chat_resolves_mutable_receiver_and_nested_calls() {
        let root = temp_project("mvp-mutable");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
struct Worker;
impl Worker {
    fn run(&mut self, _value: usize) { method_leaf(); }
}
fn inner() -> usize { free_leaf(); 1 }
fn free_leaf() {}
fn method_leaf() {}
fn exercise(mut worker: Worker) { worker.run(inner()); }
fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        assert_edge(&graph, "exercise", "Worker::run");
        assert_edge(&graph, "exercise", "inner");
        assert_edge(&graph, "inner", "free_leaf");
        assert_edge(&graph, "Worker::run", "method_leaf");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flow_mvp_chat_walks_nested_calls_in_struct_array_constructor_and_macro_args() {
        let root = temp_project("mvp-nested");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
macro_rules! pass { ($($tt:tt)*) => { $($tt)* }; }
struct Config { field: usize }
fn consume(_value: usize) {}
fn build(value: usize) -> usize { value }
fn arg_leaf() -> usize { 1 }
fn field_leaf() -> usize { 2 }
fn array_leaf() -> usize { 3 }
fn macro_leaf() -> usize { 4 }
fn exercise() {
    consume(build(arg_leaf()));
    let _config = Config { field: build(field_leaf()) };
    let _items = [array_leaf()];
    pass!(consume(build(macro_leaf())));
}
fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        for callee in [
            "consume",
            "build",
            "arg_leaf",
            "field_leaf",
            "array_leaf",
            "macro_leaf",
        ] {
            assert_edge(&graph, "exercise", callee);
        }

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flow_mvp_chat_adds_drop_edges_for_temporary_constructed_values() {
        let root = temp_project("mvp-drop-temporary");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
struct Command;
struct OrchChild(Command);
impl OrchChild {
    fn shutdown(&mut self, _wait: bool) {}
    fn consume(self) {}
}
impl Drop for OrchChild {
    fn drop(&mut self) { self.shutdown(false); }
}
fn consume(_child: OrchChild) {}
fn exercise() {
    let _items = [OrchChild(Command), OrchChild(Command)];
    (OrchChild(Command),);
    OrchChild(Command);
    consume(OrchChild(Command));
    OrchChild(Command).consume();
}
fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        assert_edge(&graph, "exercise", "OrchChild::drop");
        assert_edge(&graph, "exercise", "consume");
        assert_edge(&graph, "consume", "OrchChild::drop");
        assert_edge(&graph, "exercise", "OrchChild::consume");
        assert_edge(&graph, "OrchChild::consume", "OrchChild::drop");
        assert_edge(&graph, "OrchChild::drop", "OrchChild::shutdown");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_unqualified_sibling_call_when_suffix_is_ambiguous() {
        let root = temp_project("sibling-run");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
mod cli {
    pub fn run_from_args<I>(args: I) -> std::process::ExitCode
    where
        I: IntoIterator<Item = String>,
    {
        install_signal_handlers();
        match run(args) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("mvp-one-node-chat: {error}");
                std::process::ExitCode::from(1)
            }
        }
    }

    fn install_signal_handlers() {}

    fn run<I>(_args: I) -> Result<(), String>
    where
        I: IntoIterator<Item = String>,
    {
        Ok(())
    }
}

mod worker {
    fn run() {}
}

fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        let callees = graph
            .adjacency
            .get("cli::run_from_args")
            .expect("run_from_args should have outgoing edges");

        assert!(
            callees.contains(&"cli::run".to_string()),
            "callee list did not include sibling run(): {callees:?}"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_trait_impl_method_when_receiver_type_known() {
        let root = temp_project("trait-impl-run");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
trait Runner {
    fn run(&self);
}

struct Worker;

impl Runner for Worker {
    fn run(&self) {
        step();
    }
}

fn step() {}

fn exercise(worker: Worker) {
    worker.run();
}

fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        let callees = graph
            .adjacency
            .get("exercise")
            .expect("exercise should have outgoing edges");
        assert!(
            callees.contains(&"Worker::run".to_string()),
            "callee list did not include trait impl method: {callees:?}"
        );
        assert!(
            graph
                .adjacency
                .get("Worker::run")
                .is_some_and(|callees| callees.contains(&"step".to_string())),
            "trait method body should still resolve direct calls"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_std_read_bufread_write_impl_methods_when_receiver_type_known() {
        let root = temp_project("std-traits-run");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
use std::io::{self, BufRead, Read, Write};

struct Io;

impl Read for Io {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> { Ok(0) }
}

impl BufRead for Io {
    fn fill_buf(&mut self) -> io::Result<&[u8]> { Ok(&[]) }
    fn consume(&mut self, _amt: usize) {}
}

impl Write for Io {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> { Ok(0) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

fn exercise(mut io: Io) {
    let _ = io.read(&mut []);
    let _ = io.fill_buf();
    let _ = io.write(b"x");
    let _ = io.flush();
}

fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        let callees = graph
            .adjacency
            .get("exercise")
            .expect("exercise should have outgoing edges");
        for expected in ["Io::read", "Io::fill_buf", "Io::write", "Io::flush"] {
            assert!(
                callees.contains(&expected.to_string()),
                "callee list did not include {expected}: {callees:?}"
            );
        }

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn adds_synthetic_drop_edge_for_owned_values() {
        let root = temp_project("drop-run");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
struct Resource;

impl Drop for Resource {
    fn drop(&mut self) {
        cleanup();
    }
}

fn cleanup() {}

fn make_resource() {
    let _resource = Resource;
}

fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        assert!(
            graph
                .adjacency
                .get("make_resource")
                .is_some_and(|callees| callees.contains(&"Resource::drop".to_string())),
            "make_resource should have synthetic Resource::drop edge: {:?}",
            graph.adjacency.get("make_resource")
        );
        assert!(
            graph
                .adjacency
                .get("Resource::drop")
                .is_some_and(|callees| callees.contains(&"cleanup".to_string())),
            "drop body should still resolve cleanup"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn does_not_credit_uninvoked_closure_body() {
        let root = temp_project("closure-run");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
fn hidden() {}

fn builds_closure() {
    let _callback = || hidden();
}

fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        assert!(
            graph
                .adjacency
                .get("builds_closure")
                .is_none_or(|callees| !callees.contains(&"hidden".to_string())),
            "closure body should not be attributed to enclosing function: {:?}",
            graph.adjacency.get("builds_closure")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tracks_simple_function_pointer_parameter_seam() {
        let root = temp_project("seam-run");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
fn run(callback: fn()) {
    callback();
}

fn helper() {}

fn test_root() {
    run(helper);
}

fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        assert!(
            graph
                .adjacency
                .get("test_root")
                .is_some_and(|callees| callees.contains(&"run".to_string())),
            "test_root should call run"
        );
        assert!(
            graph
                .adjacency
                .get("run")
                .is_some_and(|callees| callees.contains(&"helper".to_string())),
            "run should inherit helper edge from callable seam: {:?}",
            graph.adjacency.get("run")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_field_receiver_inherent_methods_and_direct_helpers() {
        let root = temp_project("field-receiver-run");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
struct RuntimeConfigProfile;

impl RuntimeConfigProfile {
    fn as_str(&self) -> &'static str { "docker" }
    fn default_provider() -> ProviderKind { ProviderKind::Docker }
}

enum ProviderKind { Docker }

struct Config {
    profile: RuntimeConfigProfile,
}

fn relay_mode_env_value() -> &'static str { "1" }

impl Config {
    fn orchestrator_cli_args(&self) {
        self.profile.as_str();
        RuntimeConfigProfile::default_provider();
        relay_mode_env_value();
    }
}

fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        let callees = graph
            .adjacency
            .get("Config::orchestrator_cli_args")
            .expect("orchestrator_cli_args should have outgoing edges");
        for expected in [
            "RuntimeConfigProfile::as_str",
            "RuntimeConfigProfile::default_provider",
            "relay_mode_env_value",
        ] {
            assert!(
                callees.contains(&expected.to_string()),
                "callee list did not include {expected}: {callees:?}"
            );
        }

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_generic_trait_bound_receiver_calls_from_concrete_arguments() {
        let root = temp_project("generic-trait-run");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
trait VastAiApproval {
    fn ask(&mut self) -> bool;
}

struct StdinVastAiApproval;

impl VastAiApproval for StdinVastAiApproval {
    fn ask(&mut self) -> bool {
        confirm_vastai_if_needed()
    }
}

fn confirm_vastai_if_needed() -> bool { true }

fn confirm<P: VastAiApproval>(approval: &mut P) -> bool {
    approval.ask()
}

fn exercise() {
    let mut approval = StdinVastAiApproval;
    confirm(&mut approval);
}

fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        assert!(
            graph
                .adjacency
                .get("confirm")
                .is_some_and(|callees| callees.contains(&"StdinVastAiApproval::ask".to_string())),
            "confirm should resolve generic receiver to concrete trait impl: {:?}",
            graph.adjacency.get("confirm")
        );
        assert!(
            graph
                .adjacency
                .get("StdinVastAiApproval::ask")
                .is_some_and(|callees| callees.contains(&"confirm_vastai_if_needed".to_string())),
            "trait impl body should still resolve helper"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn adds_drop_edges_for_owned_params_and_tuple_constructors() {
        let root = temp_project("drop-param-run");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
struct OrchChild(Command);
struct Command;

impl Drop for OrchChild {
    fn drop(&mut self) {
        self.shutdown(false);
    }
}

impl OrchChild {
    fn shutdown(&mut self, _wait: bool) {}
}

fn consume(_child: OrchChild) {}

fn exercise(cmd: Command) {
    let child = OrchChild(cmd);
    consume(child);
}

fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        for caller in ["exercise", "consume"] {
            assert!(
                graph
                    .adjacency
                    .get(caller)
                    .is_some_and(|callees| callees.contains(&"OrchChild::drop".to_string())),
                "{caller} should have synthetic OrchChild::drop edge: {:?}",
                graph.adjacency.get(caller)
            );
        }
        assert!(
            graph
                .adjacency
                .get("OrchChild::drop")
                .is_some_and(|callees| callees.contains(&"OrchChild::shutdown".to_string())),
            "drop should call shutdown: {:?}",
            graph.adjacency.get("OrchChild::drop")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tracks_callbacks_registered_with_external_apis_and_closure_args() {
        let root = temp_project("callback-run");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            r#"
fn request_stop() {}
fn helper() {}

fn run(callback: impl Fn()) {
    callback();
}

fn install_signal_handler() {
    unsafe {
        libc::signal(15, request_stop as *const () as usize);
    }
}

fn exercise() {
    install_signal_handler();
    run(|| helper());
}

fn main() {}
"#,
        )
        .unwrap();

        let graph = build_call_graph(&[main], &root);
        assert!(
            graph
                .adjacency
                .get("install_signal_handler")
                .is_some_and(|callees| callees.contains(&"request_stop".to_string())),
            "external callback registration should reach request_stop: {:?}",
            graph.adjacency.get("install_signal_handler")
        );
        assert!(
            graph
                .adjacency
                .get("run")
                .is_some_and(|callees| callees.iter().any(|callee| callee.contains("{closure#"))),
            "callable parameter should reach passed closure: {:?}",
            graph.adjacency.get("run")
        );
        assert!(
            graph
                .adjacency
                .iter()
                .any(|(caller, callees)| caller.contains("{closure#")
                    && callees.contains(&"helper".to_string())),
            "closure record should reach helper: {:?}",
            graph.adjacency
        );

        fs::remove_dir_all(root).unwrap();
    }
}
