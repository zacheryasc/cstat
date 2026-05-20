//! HIR translator (spec §2.3 #11, plus the import/binding streams of
//! level 10).
//!
//! For each owner in the crate, emit one [`HirItem`] with def-path, item
//! kind, expression count, and a histogram of expression-kind strings.
//! Branch counts collapse `if`/`match`/`loop`/`while`/short-circuit
//! operators into a single scalar — same convention as the stable-side
//! AST level 8.

use std::collections::BTreeMap;

use cstat_ir::{HirDump, HirItem};
use rustc_hir as hir;
use rustc_hir::intravisit::{self, Visitor};
use rustc_middle::hir::nested_filter;
use rustc_middle::ty::TyCtxt;

pub fn run(tcx: TyCtxt<'_>) -> HirDump {
    let mut items = Vec::new();

    for local_def_id in tcx.hir_crate_items(()).definitions() {
        let def_id = local_def_id.to_def_id();
        let kind = def_kind_str(tcx.def_kind(def_id));
        let def_path = tcx.def_path_str(def_id);

        let (expr_count, branch_count, expr_kinds) = walk_body_for(tcx, local_def_id);

        items.push(HirItem {
            def_path,
            kind,
            branch_count,
            expr_count,
            expr_kinds,
        });
    }

    HirDump {
        items,
        imports: Vec::new(),
        bindings: Vec::new(),
    }
}

fn walk_body_for(
    tcx: TyCtxt<'_>,
    local_def_id: rustc_hir::def_id::LocalDefId,
) -> (u32, u32, BTreeMap<String, u32>) {
    let Some(body_id) = tcx.hir_maybe_body_owned_by(local_def_id) else {
        return (0, 0, BTreeMap::new());
    };
    let mut v = ExprWalker {
        tcx,
        expr_count: 0,
        branch_count: 0,
        kinds: BTreeMap::new(),
    };
    v.visit_body(body_id);
    (v.expr_count, v.branch_count, v.kinds)
}

struct ExprWalker<'tcx> {
    tcx: TyCtxt<'tcx>,
    expr_count: u32,
    branch_count: u32,
    kinds: BTreeMap<String, u32>,
}

impl<'tcx> Visitor<'tcx> for ExprWalker<'tcx> {
    type NestedFilter = nested_filter::OnlyBodies;
    type MaybeTyCtxt = TyCtxt<'tcx>;

    fn maybe_tcx(&mut self) -> Self::MaybeTyCtxt {
        self.tcx
    }

    fn visit_expr(&mut self, ex: &'tcx hir::Expr<'tcx>) {
        self.expr_count += 1;
        let kind = expr_kind_str(&ex.kind);
        if is_branching_kind(kind) {
            self.branch_count += 1;
        }
        *self.kinds.entry(kind.into()).or_insert(0) += 1;
        intravisit::walk_expr(self, ex);
    }
}

fn is_branching_kind(k: &str) -> bool {
    matches!(k, "if" | "match" | "loop" | "while" | "and_and" | "or_or")
}

fn expr_kind_str(k: &hir::ExprKind<'_>) -> &'static str {
    use hir::ExprKind::*;
    match k {
        Lit(_) => "lit",
        Path(_) => "path",
        Call(..) => "call",
        MethodCall(..) => "method_call",
        If(..) => "if",
        Match(..) => "match",
        Loop(..) => "loop",
        Block(..) => "block",
        Binary(op, _, _) => match op.node {
            hir::BinOpKind::And => "and_and",
            hir::BinOpKind::Or => "or_or",
            _ => "binary",
        },
        Unary(..) => "unary",
        Tup(..) => "tuple",
        Field(..) => "field",
        Index(..) => "index",
        Cast(..) => "cast",
        AddrOf(..) => "addr_of",
        Assign(..) => "assign",
        AssignOp(..) => "assign_op",
        Closure(..) => "closure",
        Array(..) => "array",
        Struct(..) => "struct",
        Repeat(..) => "repeat",
        Yield(..) => "yield",
        Become(..) => "become",
        Break(..) => "break",
        Continue(..) => "continue",
        Ret(..) => "ret",
        Let(..) => "let",
        DropTemps(..) => "drop_temps",
        ConstBlock(..) => "const_block",
        InlineAsm(..) => "inline_asm",
        OffsetOf(..) => "offset_of",
        Type(..) => "type_ascription",
        Err(..) => "err",
        _ => "other",
    }
}

fn def_kind_str(k: rustc_hir::def::DefKind) -> String {
    use rustc_hir::def::DefKind;
    match k {
        DefKind::Mod => "module".into(),
        DefKind::Struct => "struct".into(),
        DefKind::Union => "union".into(),
        DefKind::Enum => "enum".into(),
        DefKind::Variant => "variant".into(),
        DefKind::Trait => "trait".into(),
        DefKind::TyAlias => "type_alias".into(),
        DefKind::ForeignTy => "foreign_ty".into(),
        DefKind::TraitAlias => "trait_alias".into(),
        DefKind::AssocTy => "assoc_type".into(),
        DefKind::TyParam => "type_param".into(),
        DefKind::Fn => "fn".into(),
        DefKind::Const { .. } => "const".into(),
        DefKind::ConstParam => "const_param".into(),
        DefKind::Static { .. } => "static".into(),
        DefKind::Ctor(..) => "constructor".into(),
        DefKind::AssocFn => "method".into(),
        DefKind::AssocConst { .. } => "assoc_const".into(),
        DefKind::Macro(..) => "macro".into(),
        DefKind::ExternCrate => "extern_crate".into(),
        DefKind::Use => "use".into(),
        DefKind::ForeignMod => "foreign_mod".into(),
        DefKind::AnonConst => "anon_const".into(),
        DefKind::InlineConst => "inline_const".into(),
        DefKind::OpaqueTy => "opaque_type".into(),
        DefKind::Field => "field".into(),
        DefKind::LifetimeParam => "lifetime_param".into(),
        DefKind::GlobalAsm => "global_asm".into(),
        DefKind::Impl { .. } => "impl".into(),
        DefKind::Closure => "closure".into(),
        DefKind::SyntheticCoroutineBody => "synthetic_coroutine_body".into(),
    }
}
