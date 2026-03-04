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
}

/// Extracted information about a trait.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TraitInfo {
    pub name: String,
    pub file: PathBuf,
    pub method_count: usize,
    pub generic_param_count: usize,
}

/// Extracted information about an impl block.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ImplInfo {
    pub target_type: String,
    pub file: PathBuf,
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
}

impl SymbolExtractor {
    fn extract_item(&mut self, item: &Item) {
        match item {
            Item::Fn(func) => {
                let info = extract_function_info(
                    &func.sig.ident.to_string(),
                    &self.file_path,
                    &func.sig,
                    Some(&func.block),
                );
                self.functions.push(info);
            }
            Item::Struct(s) => {
                let field_count = match &s.fields {
                    syn::Fields::Named(f) => f.named.len(),
                    syn::Fields::Unnamed(f) => f.unnamed.len(),
                    syn::Fields::Unit => 0,
                };
                let derive_count = count_derives(&s.attrs);
                let generic_param_count = s.generics.params.len();
                self.structs.push(StructInfo {
                    name: s.ident.to_string(),
                    file: self.file_path.clone(),
                    field_count,
                    derive_count,
                    generic_param_count,
                });
            }
            Item::Enum(e) => {
                let derive_count = count_derives(&e.attrs);
                let generic_param_count = e.generics.params.len();
                self.enums.push(EnumInfo {
                    name: e.ident.to_string(),
                    file: self.file_path.clone(),
                    variant_count: e.variants.len(),
                    derive_count,
                    generic_param_count,
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
                    name: t.ident.to_string(),
                    file: self.file_path.clone(),
                    method_count,
                    generic_param_count,
                });
            }
            Item::Impl(imp) => {
                let target_type = type_to_string(&imp.self_ty);
                let trait_name = imp.trait_.as_ref().map(|(_, path, _)| {
                    path.segments
                        .iter()
                        .map(|s| s.ident.to_string())
                        .collect::<Vec<_>>()
                        .join("::")
                });
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
                        );
                        self.functions.push(info);
                    }
                }
                self.impls.push(ImplInfo {
                    target_type,
                    file: self.file_path.clone(),
                    method_count,
                    trait_name,
                });
            }
            _ => {}
        }
    }
}

fn extract_function_info(
    name: &str,
    file: &Path,
    sig: &syn::Signature,
    body: Option<&syn::Block>,
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
    }
}

/// Count derives from attributes.
fn count_derives(attrs: &[syn::Attribute]) -> usize {
    attrs
        .iter()
        .filter(|a| a.path().is_ident("derive"))
        .count()
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

/// Convert a Type to a readable string (public wrapper for cross-module use).
pub fn type_to_string_pub(ty: &Type) -> String {
    type_to_string(ty)
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
