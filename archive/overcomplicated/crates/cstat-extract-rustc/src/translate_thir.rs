//! THIR-equivalent translator (spec §2.3 #12).
//!
//! THIR is HIR plus type information. By the time `after_analysis` fires
//! the THIR has been stolen by borrow-check, so we reconstitute the same
//! information by walking HIR with `tcx.typeck(body_owner)` in hand —
//! the per-expression type table is exactly what THIR carries. This
//! gives us the level-12 metrics (distinct types per body, generic-arg
//! density, trait-object density) without needing to access THIR
//! storage directly.

use std::collections::BTreeSet;

use cstat_ir::{ThirBody, ThirDump};
use rustc_hir as hir;
use rustc_hir::intravisit::{self, Visitor};
use rustc_middle::hir::nested_filter;
use rustc_middle::ty::{Ty, TyCtxt, TyKind, TypeVisitableExt, TypeckResults};

pub fn run(tcx: TyCtxt<'_>) -> ThirDump {
    let mut bodies = Vec::new();
    for local_def_id in tcx.hir_body_owners() {
        if !tcx.has_typeck_results(local_def_id) {
            continue;
        }
        let typeck = tcx.typeck(local_def_id);
        let Some(body_id) = tcx.hir_maybe_body_owned_by(local_def_id) else {
            continue;
        };
        let mut v = ThirWalker {
            tcx,
            typeck,
            expr_count: 0,
            distinct_types: BTreeSet::new(),
            generic_arg_total: 0,
            trait_object_count: 0,
            generic_expr_count: 0,
        };
        v.visit_body(body_id);
        bodies.push(ThirBody {
            def_path: tcx.def_path_str(local_def_id.to_def_id()),
            expr_count: v.expr_count,
            distinct_type_count: v.distinct_types.len() as u32,
            generic_arg_total: v.generic_arg_total,
            trait_object_count: v.trait_object_count,
            generic_expr_count: v.generic_expr_count,
        });
    }
    ThirDump { bodies }
}

struct ThirWalker<'tcx> {
    tcx: TyCtxt<'tcx>,
    typeck: &'tcx TypeckResults<'tcx>,
    expr_count: u32,
    distinct_types: BTreeSet<String>,
    generic_arg_total: u32,
    trait_object_count: u32,
    generic_expr_count: u32,
}

impl<'tcx> Visitor<'tcx> for ThirWalker<'tcx> {
    type NestedFilter = nested_filter::OnlyBodies;
    type MaybeTyCtxt = TyCtxt<'tcx>;

    fn maybe_tcx(&mut self) -> Self::MaybeTyCtxt {
        self.tcx
    }

    fn visit_expr(&mut self, ex: &'tcx hir::Expr<'tcx>) {
        self.expr_count += 1;
        if let Some(ty) = self.typeck.expr_ty_opt(ex) {
            self.note_type(ty);
        }
        intravisit::walk_expr(self, ex);
    }
}

impl<'tcx> ThirWalker<'tcx> {
    fn note_type(&mut self, ty: Ty<'tcx>) {
        let s = format!("{ty}");
        self.distinct_types.insert(s);

        // Generic-arg total: count the top-level GenericArg list on
        // type constructors that carry one (Adt, Foreign, Closure, etc.).
        let (generic_arg_count, has_param, is_dyn) = inspect_ty(ty);
        self.generic_arg_total = self.generic_arg_total.saturating_add(generic_arg_count);
        if has_param {
            self.generic_expr_count = self.generic_expr_count.saturating_add(1);
        }
        if is_dyn {
            self.trait_object_count = self.trait_object_count.saturating_add(1);
        }
    }
}

fn inspect_ty<'tcx>(ty: Ty<'tcx>) -> (u32, bool, bool) {
    let mut generic_arg_count: u32 = 0;
    if let TyKind::Adt(_def, args) = ty.kind() {
        generic_arg_count = args.len() as u32;
    }
    let has_param = ty.has_param();
    let is_dyn = matches!(ty.kind(), TyKind::Dynamic(..));
    (generic_arg_count, has_param, is_dyn)
}
