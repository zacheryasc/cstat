//! MIR translator (spec §2.3 #13 + #14).
//!
//! Reads `optimized_mir` for every MIR-bearing body owner and emits one
//! [`MirBody`] per body with the basic-block graph, statement-kind
//! strings, terminator-kind strings, and predecessor/successor edges.
//!
//! Pre/post opt distinction: in `after_analysis` we can only see one
//! MIR snapshot reliably because `mir_built` has already been stolen
//! by borrow-check. The caller (Stage 13 orchestrator) re-invokes this
//! binary twice with different `-Zmir-opt-level` settings when it
//! wants a delta; the wire format already holds both snapshots.

use cstat_ir::{MirBasicBlock, MirBody, MirDump};
use rustc_hir::def_id::DefId;
use rustc_middle::mir;
use rustc_middle::ty::TyCtxt;

pub fn run(tcx: TyCtxt<'_>) -> MirDump {
    let mut bodies = Vec::new();
    for local_def_id in tcx.hir_body_owners() {
        let def_id = local_def_id.to_def_id();
        if !tcx.is_mir_available(def_id) {
            continue;
        }
        let body = tcx.optimized_mir(def_id);
        bodies.push(translate_body(tcx, body, def_id));
    }
    MirDump { bodies }
}

fn translate_body<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &'tcx mir::Body<'tcx>,
    def_id: DefId,
) -> MirBody {
    let predecessors = body.basic_blocks.predecessors();
    let mut bbs = Vec::with_capacity(body.basic_blocks.len());
    for (bb_idx, bb_data) in body.basic_blocks.iter_enumerated() {
        let statement_kinds: Vec<String> = bb_data
            .statements
            .iter()
            .map(|s| stmt_kind_str(&s.kind).to_string())
            .collect();
        let (terminator, successors) = match &bb_data.terminator {
            Some(term) => {
                let succs: Vec<u32> = term.successors().map(|s| s.as_u32()).collect();
                (terminator_kind_str(&term.kind).to_string(), succs)
            }
            None => ("MissingTerminator".to_string(), Vec::new()),
        };
        let preds: Vec<u32> = predecessors[bb_idx].iter().map(|p| p.as_u32()).collect();
        bbs.push(MirBasicBlock {
            id: bb_idx.as_u32(),
            statement_kinds,
            terminator,
            predecessors: preds,
            successors,
        });
    }
    MirBody {
        def_path: tcx.def_path_str(def_id),
        arg_count: body.arg_count as u32,
        local_count: body.local_decls.len() as u32,
        basic_blocks: bbs,
    }
}

fn stmt_kind_str(k: &mir::StatementKind<'_>) -> &'static str {
    use mir::StatementKind::*;
    match k {
        Assign(..) => "Assign",
        FakeRead(..) => "FakeRead",
        SetDiscriminant { .. } => "SetDiscriminant",
        StorageLive(..) => "StorageLive",
        StorageDead(..) => "StorageDead",
        PlaceMention(..) => "PlaceMention",
        AscribeUserType(..) => "AscribeUserType",
        Coverage(..) => "Coverage",
        Intrinsic(..) => "Intrinsic",
        ConstEvalCounter => "ConstEvalCounter",
        Nop => "Nop",
        BackwardIncompatibleDropHint { .. } => "BackwardIncompatibleDropHint",
    }
}

fn terminator_kind_str(k: &mir::TerminatorKind<'_>) -> &'static str {
    use mir::TerminatorKind::*;
    match k {
        Goto { .. } => "Goto",
        SwitchInt { .. } => "SwitchInt",
        UnwindResume => "UnwindResume",
        UnwindTerminate(..) => "UnwindTerminate",
        Return => "Return",
        Unreachable => "Unreachable",
        Drop { .. } => "Drop",
        Call { .. } => "Call",
        TailCall { .. } => "TailCall",
        Assert { .. } => "Assert",
        Yield { .. } => "Yield",
        CoroutineDrop => "CoroutineDrop",
        FalseEdge { .. } => "FalseEdge",
        FalseUnwind { .. } => "FalseUnwind",
        InlineAsm { .. } => "InlineAsm",
    }
}
