//! `rustc_driver::Callbacks` hook. Runs after `analysis` and dispatches
//! to each per-IR translator.
//!
//! Borrow-check has already consumed `mir_built` by the time
//! `after_analysis` fires, which is why both pre-opt and post-opt MIR
//! sections currently read from `optimized_mir`. Stage 13 will refine
//! this — the orchestrator can invoke this binary twice with
//! `-Zmir-opt-level=0` and the default level if it wants a true pre/post
//! diff. The wire format already supports both snapshots; what's
//! constrained today is the *source* of the pre-opt snapshot, not the
//! shape of the dump.

use cstat_ir::{CstatIrDump, ToolchainStamp};
use rustc_driver::Compilation;
use rustc_interface::interface::Compiler;
use rustc_middle::ty::TyCtxt;

pub struct ExtractCallbacks {
    toolchain: ToolchainStamp,
    dump: Option<CstatIrDump>,
}

impl ExtractCallbacks {
    pub fn new(toolchain: ToolchainStamp) -> Self {
        Self {
            toolchain,
            dump: None,
        }
    }

    pub fn into_dump(self) -> CstatIrDump {
        self.dump
            .unwrap_or_else(|| CstatIrDump::empty(self.toolchain))
    }

    fn translate<'tcx>(&self, tcx: TyCtxt<'tcx>) -> CstatIrDump {
        let mut dump = CstatIrDump::empty(self.toolchain.clone());
        dump.hir = Some(crate::translate_hir::run(tcx));
        dump.thir = Some(crate::translate_thir::run(tcx));
        let mir = crate::translate_mir::run(tcx);
        dump.mir_preopt = Some(mir.clone());
        dump.mir_opt = Some(mir);
        dump.borrowck = Some(crate::translate_borrowck::run(tcx));
        dump.mono_items = Some(crate::translate_mono::run(tcx));
        dump
    }
}

impl rustc_driver::Callbacks for ExtractCallbacks {
    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        let dump = self.translate(tcx);
        self.dump = Some(dump);
        // Stop the compiler — we have what we need and don't want it
        // to spend wall-clock running codegen we already disabled via
        // `--emit=metadata`. `Stop` is also what tells the driver to
        // skip downstream queries that might error on partial state.
        Compilation::Stop
    }
}
