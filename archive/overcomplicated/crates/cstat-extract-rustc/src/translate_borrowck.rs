//! Borrow-check artifact translator (spec §2.3 #15).
//!
//! `tcx.mir_borrowck(...)` is a private query; we cannot pull the full
//! NLL constraint graph from a `Callbacks` impl. v1 reads the
//! observable artifacts on `optimized_mir` instead:
//!
//! - `region_count` counts MIR local decls whose type carries a free
//!   or late-bound region. That tracks `&'a T`-style locals which are
//!   the dominant region-bearing entities in user code.
//! - `outlives_constraint_count`, `member_constraint_count`,
//!   `nll_conflict_count` start at zero. They populate when a future
//!   stage either gains access to the private NLL query or routes the
//!   data through an emitted artifact rustc surfaces stably.
//!
//! The wire format is the load-bearing thing — level 15 reads exactly
//! these scalar fields. Filling them out is a translator change, not a
//! schema change.

use cstat_ir::{BorrowckBody, BorrowckDump};
use rustc_middle::ty::{Ty, TyCtxt, TypeVisitableExt};

pub fn run(tcx: TyCtxt<'_>) -> BorrowckDump {
    let mut bodies = Vec::new();
    for local_def_id in tcx.hir_body_owners() {
        let def_id = local_def_id.to_def_id();
        if !tcx.is_mir_available(def_id) {
            continue;
        }
        let body = tcx.optimized_mir(def_id);
        let region_count = body
            .local_decls
            .iter()
            .filter(|d| has_region(d.ty))
            .count() as u32;
        bodies.push(BorrowckBody {
            def_path: tcx.def_path_str(def_id),
            region_count,
            outlives_constraint_count: 0,
            member_constraint_count: 0,
            nll_conflict_count: 0,
        });
    }
    BorrowckDump { bodies }
}

fn has_region<'tcx>(ty: Ty<'tcx>) -> bool {
    ty.has_free_regions() || ty.has_bound_regions() || ty.has_erased_regions()
}
