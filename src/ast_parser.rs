use std::path::{Path, PathBuf};
use syn::visit::Visit;
use syn::{
    Expr, File as SynFile, GenericParam, ImplItem, Item, ReturnType, TraitItem, Type,
    WherePredicate,
};

/// Extracted information about a function (free function or method).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FunctionInfo {
    pub name: String,
    pub file: PathBuf,
    pub param_count: usize,
    pub nesting_depth: usize,
    pub branch_points: usize,
    pub body_stmt_count: usize,
    pub generic_param_count: usize,
    pub trait_bound_count: usize,
    pub where_predicate_count: usize,
    pub return_type_complexity: usize,
    pub line_start: usize,
    pub line_end: usize,
    pub is_test_support: bool,
}

/// Extracted information about a struct.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StructInfo {
    pub name: String,
    pub file: PathBuf,
    pub field_count: usize,
    pub derive_count: usize,
    pub generic_param_count: usize,
    pub line_start: usize,
    pub line_end: usize,
}

/// Extracted information about an enum.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct EnumInfo {
    pub name: String,
    pub file: PathBuf,
    pub variant_count: usize,
    pub derive_count: usize,
    pub generic_param_count: usize,
    pub line_start: usize,
    pub line_end: usize,
}

/// Extracted information about a trait.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TraitInfo {
    pub name: String,
    pub file: PathBuf,
    pub method_count: usize,
    pub generic_param_count: usize,
    pub line_start: usize,
    pub line_end: usize,
}

/// Extracted information about a const item.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ConstInfo {
    pub name: String,
    pub file: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
}

/// Extracted information about a static item.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StaticInfo {
    pub name: String,
    pub file: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
}

/// Extracted information about an impl block.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ImplInfo {
    pub target_type: String,
    pub file: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub method_count: usize,
    pub trait_name: Option<String>,
}

/// All extracted symbols from a single file.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FileSymbols {
    pub path: PathBuf,
    pub functions: Vec<FunctionInfo>,
    pub structs: Vec<StructInfo>,
    pub enums: Vec<EnumInfo>,
    pub traits: Vec<TraitInfo>,
    pub impls: Vec<ImplInfo>,
    pub consts: Vec<ConstInfo>,
    pub statics: Vec<StaticInfo>,
    pub parse_error: bool,
}

/// All extracted symbols across the entire project.
pub struct ProjectSymbols {
    pub files: Vec<FileSymbols>,
}

impl ProjectSymbols {
    #[allow(dead_code)]
    pub fn all_functions(&self) -> Vec<&FunctionInfo> {
        self.files.iter().flat_map(|f| f.functions.iter()).collect()
    }
}

/// Parse all given Rust source files and extract symbols.
pub fn parse_project(rs_files: &[PathBuf]) -> ProjectSymbols {
    let files: Vec<FileSymbols> = rs_files.iter().map(|p| parse_file(p)).collect();
    ProjectSymbols { files }
}

/// Parse a single Rust file and extract its symbols.
fn parse_file(path: &Path) -> FileSymbols {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            return FileSymbols {
                path: path.to_path_buf(),
                functions: vec![],
                structs: vec![],
                enums: vec![],
                traits: vec![],
                impls: vec![],
                consts: vec![],
                statics: vec![],
                parse_error: true,
            };
        }
    };

    let ast: SynFile = match syn::parse_file(&content) {
        Ok(f) => f,
        Err(_) => {
            return FileSymbols {
                path: path.to_path_buf(),
                functions: vec![],
                structs: vec![],
                enums: vec![],
                traits: vec![],
                impls: vec![],
                consts: vec![],
                statics: vec![],
                parse_error: true,
            };
        }
    };

    let mut extractor = SymbolExtractor {
        file_path: path.to_path_buf(),
        functions: vec![],
        structs: vec![],
        enums: vec![],
        traits: vec![],
        impls: vec![],
        consts: vec![],
        statics: vec![],
        module_parts: vec![],
        in_cfg_test: false,
        in_tests_module: false,
    };

    for item in &ast.items {
        extractor.extract_item(item);
    }

    FileSymbols {
        path: path.to_path_buf(),
        functions: extractor.functions,
        structs: extractor.structs,
        enums: extractor.enums,
        traits: extractor.traits,
        impls: extractor.impls,
        consts: extractor.consts,
        statics: extractor.statics,
        parse_error: false,
    }
}

struct SymbolExtractor {
    file_path: PathBuf,
    functions: Vec<FunctionInfo>,
    structs: Vec<StructInfo>,
    enums: Vec<EnumInfo>,
    traits: Vec<TraitInfo>,
    impls: Vec<ImplInfo>,
    consts: Vec<ConstInfo>,
    statics: Vec<StaticInfo>,
    module_parts: Vec<String>,
    in_cfg_test: bool,
    in_tests_module: bool,
}

impl SymbolExtractor {
    fn extract_item(&mut self, item: &Item) {
        match item {
            Item::Fn(func) => {
                let name = self.qualify_name(&func.sig.ident.to_string());
                let is_test_support =
                    self.is_test_support_context() || has_cfg_test_attr(&func.attrs);
                let info = extract_function_info(
                    &name,
                    &self.file_path,
                    &func.sig,
                    Some(&func.block),
                    is_test_support,
                );
                self.functions.push(info);
            }
            Item::Struct(s) => {
                let field_count = match &s.fields {
                    syn::Fields::Named(f) => f.named.len(),
                    syn::Fields::Unnamed(f) => f.unnamed.len(),
                    syn::Fields::Unit => 0,
                };
                let line_start = s.struct_token.span.start().line;
                let line_end = match &s.fields {
                    syn::Fields::Named(fields) => fields.brace_token.span.close().end().line,
                    syn::Fields::Unnamed(fields) => fields.paren_token.span.close().end().line,
                    syn::Fields::Unit => s
                        .semi_token
                        .as_ref()
                        .map_or(line_start, |semi| semi.span.end().line),
                };
                let derive_count = count_derives(&s.attrs);
                let generic_param_count = s.generics.params.len();
                self.structs.push(StructInfo {
                    name: self.qualify_name(&s.ident.to_string()),
                    file: self.file_path.clone(),
                    field_count,
                    derive_count,
                    generic_param_count,
                    line_start,
                    line_end,
                });
            }
            Item::Enum(e) => {
                let derive_count = count_derives(&e.attrs);
                let generic_param_count = e.generics.params.len();
                self.enums.push(EnumInfo {
                    name: self.qualify_name(&e.ident.to_string()),
                    file: self.file_path.clone(),
                    variant_count: e.variants.len(),
                    derive_count,
                    generic_param_count,
                    line_start: e.enum_token.span.start().line,
                    line_end: e.brace_token.span.close().end().line,
                });
            }
            Item::Trait(t) => {
                let method_count = t
                    .items
                    .iter()
                    .filter(|i| matches!(i, TraitItem::Fn(_)))
                    .count();
                let generic_param_count = t.generics.params.len();
                self.traits.push(TraitInfo {
                    name: self.qualify_name(&t.ident.to_string()),
                    file: self.file_path.clone(),
                    method_count,
                    generic_param_count,
                    line_start: t.trait_token.span.start().line,
                    line_end: t.brace_token.span.close().end().line,
                });
            }
            Item::Const(c) => {
                self.consts.push(ConstInfo {
                    name: self.qualify_name(&c.ident.to_string()),
                    file: self.file_path.clone(),
                    line_start: c.const_token.span.start().line,
                    line_end: c.semi_token.span.end().line,
                });
            }
            Item::Static(s) => {
                self.statics.push(StaticInfo {
                    name: self.qualify_name(&s.ident.to_string()),
                    file: self.file_path.clone(),
                    line_start: s.static_token.span.start().line,
                    line_end: s.semi_token.span.end().line,
                });
            }
            Item::Impl(imp) => {
                let target_type = self.qualify_name(&type_to_string(&imp.self_ty));
                let trait_name = imp.trait_.as_ref().map(|(_, path, _)| {
                    path.segments
                        .iter()
                        .map(|s| s.ident.to_string())
                        .collect::<Vec<_>>()
                        .join("::")
                });
                let line_start = imp.impl_token.span.start().line;
                let line_end = imp.brace_token.span.close().end().line;
                let mut method_count = 0;
                for impl_item in &imp.items {
                    if let ImplItem::Fn(method) = impl_item {
                        method_count += 1;
                        let qualified_name = format!("{}::{}", target_type, method.sig.ident);
                        let info = extract_function_info(
                            &qualified_name,
                            &self.file_path,
                            &method.sig,
                            Some(&method.block),
                            self.is_test_support_context()
                                || has_cfg_test_attr(&method.attrs)
                                || has_cfg_test_attr(&imp.attrs),
                        );
                        self.functions.push(info);
                    }
                }
                self.impls.push(ImplInfo {
                    target_type,
                    file: self.file_path.clone(),
                    line_start,
                    line_end,
                    method_count,
                    trait_name,
                });
            }
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    let previous_cfg = self.in_cfg_test;
                    let previous_tests = self.in_tests_module;
                    self.in_cfg_test |= has_cfg_test_attr(&module.attrs);
                    self.in_tests_module |= module.ident == "tests";
                    self.module_parts.push(module.ident.to_string());
                    for item in items {
                        self.extract_item(item);
                    }
                    self.module_parts.pop();
                    self.in_cfg_test = previous_cfg;
                    self.in_tests_module = previous_tests;
                }
            }
            _ => {}
        }
    }

    fn qualify_name(&self, name: &str) -> String {
        if self.module_parts.is_empty() {
            name.to_string()
        } else {
            format!("{}::{}", self.module_parts.join("::"), name)
        }
    }

    fn is_test_support_context(&self) -> bool {
        self.in_cfg_test || self.in_tests_module
    }
}

fn extract_function_info(
    name: &str,
    file: &Path,
    sig: &syn::Signature,
    body: Option<&syn::Block>,
    is_test_support: bool,
) -> FunctionInfo {
    let param_count = sig.inputs.len();
    let generic_param_count = sig.generics.params.len();
    let trait_bound_count = count_generic_trait_bounds(&sig.generics);
    let where_predicate_count = sig
        .generics
        .where_clause
        .as_ref()
        .map_or(0, |w| w.predicates.len());
    let return_type_complexity = match &sig.output {
        ReturnType::Default => 0,
        ReturnType::Type(_, ty) => type_complexity(ty),
    };

    let (nesting_depth, branch_points, body_stmt_count) = if let Some(block) = body {
        let stmts = block.stmts.len();
        let mut visitor = ControlFlowVisitor::new();
        visitor.visit_block(block);
        (visitor.max_depth, visitor.branch_points, stmts)
    } else {
        (0, 0, 0)
    };

    // Get line span from the function signature and body
    let line_start = sig.fn_token.span.start().line;
    let line_end = if let Some(block) = body {
        block.brace_token.span.close().end().line
    } else {
        line_start
    };

    FunctionInfo {
        name: name.to_string(),
        file: file.to_path_buf(),
        param_count,
        nesting_depth,
        branch_points,
        body_stmt_count,
        generic_param_count,
        trait_bound_count,
        where_predicate_count,
        return_type_complexity,
        line_start,
        line_end,
        is_test_support,
    }
}

/// Count derives from attributes.
fn count_derives(attrs: &[syn::Attribute]) -> usize {
    attrs.iter().filter(|a| a.path().is_ident("derive")).count()
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

/// Count trait bounds across all generic parameters.
fn count_generic_trait_bounds(generics: &syn::Generics) -> usize {
    let mut count = 0;
    for param in &generics.params {
        if let GenericParam::Type(tp) = param {
            count += tp.bounds.len();
        }
    }
    if let Some(where_clause) = &generics.where_clause {
        for pred in &where_clause.predicates {
            if let WherePredicate::Type(pt) = pred {
                count += pt.bounds.len();
            }
        }
    }
    count
}

/// Estimate the "complexity" of a type — how deeply nested / how many components.
fn type_complexity(ty: &Type) -> usize {
    match ty {
        Type::Path(tp) => {
            let mut c = 1;
            for seg in &tp.path.segments {
                if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                    for arg in &args.args {
                        if let syn::GenericArgument::Type(inner) = arg {
                            c += type_complexity(inner);
                        }
                    }
                }
            }
            c
        }
        Type::Reference(r) => 1 + type_complexity(&r.elem),
        Type::Tuple(t) => 1 + t.elems.iter().map(type_complexity).sum::<usize>(),
        Type::Slice(s) => 1 + type_complexity(&s.elem),
        Type::Array(a) => 1 + type_complexity(&a.elem),
        Type::ImplTrait(i) => 1 + i.bounds.len(),
        Type::TraitObject(t) => 1 + t.bounds.len(),
        _ => 1,
    }
}

/// Convert a Type to a readable string.
fn type_to_string(ty: &Type) -> String {
    match ty {
        Type::Path(tp) => tp
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        _ => "?".to_string(),
    }
}

/// AST visitor that walks control flow to compute nesting depth and branch points.
struct ControlFlowVisitor {
    current_depth: usize,
    max_depth: usize,
    branch_points: usize,
}

impl ControlFlowVisitor {
    fn new() -> Self {
        Self {
            current_depth: 0,
            max_depth: 0,
            branch_points: 0,
        }
    }

    fn enter_nesting(&mut self) {
        self.current_depth += 1;
        if self.current_depth > self.max_depth {
            self.max_depth = self.current_depth;
        }
    }

    fn exit_nesting(&mut self) {
        self.current_depth -= 1;
    }
}

impl<'ast> Visit<'ast> for ControlFlowVisitor {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        match expr {
            Expr::If(_) => {
                self.branch_points += 1;
                self.enter_nesting();
                syn::visit::visit_expr(self, expr);
                self.exit_nesting();
                return;
            }
            Expr::Match(m) => {
                // Each arm beyond the first is a branch point
                self.branch_points += m.arms.len().saturating_sub(1);
                self.enter_nesting();
                syn::visit::visit_expr(self, expr);
                self.exit_nesting();
                return;
            }
            Expr::While(_) | Expr::ForLoop(_) | Expr::Loop(_) => {
                self.branch_points += 1;
                self.enter_nesting();
                syn::visit::visit_expr(self, expr);
                self.exit_nesting();
                return;
            }
            Expr::Closure(_) => {
                self.enter_nesting();
                syn::visit::visit_expr(self, expr);
                self.exit_nesting();
                return;
            }
            _ => {}
        }
        syn::visit::visit_expr(self, expr);
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        if arm.guard.is_some() {
            self.branch_points += 1;
        }
        syn::visit::visit_arm(self, arm);
    }
}
