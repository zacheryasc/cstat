//! Stable-Rust `Level` implementations.
//!
//! Stage 3 wires spec §2.1 (levels 1–6) and the rustfmt-diff level from
//! §2.9 (level 47). Later stages will populate the remaining stable-side
//! levels (AST, cargo metadata, codegen, etc.). The orchestrator-owned
//! "every required level is reported" contract means anything we have not
//! registered here automatically appears as `skipped: not implemented`.

mod ast_cache;
mod ast_levels;
mod binary_levels;
mod cargo_levels;
mod clippy_level;
mod codegen_levels;
mod derived_levels;
mod fmt_levels;
mod fs_levels;
mod graph_cache;
mod graph_levels;
mod helpers;
mod inventory;
mod ir_cache;
mod llvm_model;
mod rustc_levels;
mod rustdoc_cache;
mod rustdoc_level;
mod test_levels;
mod tok_levels;
mod type_levels;

use cstat_orchestrate::Runner;

use crate::ast_cache::SharedAst;
use crate::ast_levels::{AstCstLevel, AstExpandedLevel, AstItemsLevel};
use crate::binary_levels::{
    DwarfLevel, LinkMapLevel, SharedBinary, SymDemangledLevel, SymRelocsLevel, SymSectionsLevel,
    SymTableLevel,
};
use crate::cargo_levels::{
    CargoCfgLevel, CargoLockLevel, CargoToolchainLevel, CargoWorkspaceLevel,
};
use crate::clippy_level::ClippyLevel;
use crate::codegen_levels::{
    AsmLevel, BinLevel, LlvmOptLevel, LlvmPreoptLevel, ObjLevel, SharedCodegen,
};
use crate::derived_levels::{GraphCallMonoLevel, GraphDataflowLevel};
use crate::llvm_model::SharedLlvmModel;
use crate::fmt_levels::{FmtWhitespaceLevel, RustfmtLevel};
use crate::fs_levels::{FsBytesLevel, FsLayoutLevel, FsSourceLevel};
use crate::graph_cache::SharedGraphs;
use crate::graph_levels::{
    GraphCallStaticLevel, GraphCfgLevel, GraphCommunitiesLevel, GraphModLevel, GraphPowerLawLevel,
    GraphSccsLevel, GraphSelfSimLevel, GraphTypeUsageLevel,
};
use crate::inventory::SharedInventory;
use crate::ir_cache::SharedIrDump;
use crate::rustc_levels::{
    AstResolvedLevel, BorrowckLevel, HirLevel, MirOptLevel, MirPreoptLevel, ThirLevel,
    TypeMonoLevel,
};
use crate::rustdoc_cache::SharedRustdoc;
use crate::rustdoc_level::RustdocLevel;
use crate::test_levels::{DoctestsLevel, TestsLevel};
use crate::tok_levels::{TokCommentsLevel, TokStreamLevel};
use crate::type_levels::{TypeCoherenceLevel, TypeTraitImplLevel, TypeUsageLevel};

/// Register every stable-Rust extractor that has a working implementation.
///
/// All filesystem / source-text levels share a single lazy inventory so the
/// project tree is walked at most once per run; the AST levels share a
/// lazy `syn::parse_file` cache for the same reason.
pub fn register_all_stable(runner: &mut Runner) {
    let shared = SharedInventory::new();
    let ast = SharedAst::new(shared.clone());
    let codegen = SharedCodegen::new();
    let binary = SharedBinary::new(codegen.clone(), ast.clone());
    let rustdoc = SharedRustdoc::new();
    let graphs = SharedGraphs::new(shared.clone(), ast.clone());
    let ir = SharedIrDump::new();
    let llvm_model = SharedLlvmModel::new(codegen.clone());
    runner
        .register(FsLayoutLevel { shared: shared.clone() })
        .register(FsBytesLevel { shared: shared.clone() })
        .register(FsSourceLevel { shared: shared.clone() })
        .register(TokStreamLevel { shared: shared.clone() })
        .register(TokCommentsLevel { shared: shared.clone() })
        .register(FmtWhitespaceLevel { shared })
        .register(RustfmtLevel)
        .register(AstCstLevel { shared: ast.clone() })
        .register(AstItemsLevel { shared: ast.clone() })
        .register(AstExpandedLevel { shared: ast.clone() })
        .register(CargoWorkspaceLevel)
        .register(CargoLockLevel)
        .register(CargoCfgLevel)
        .register(CargoToolchainLevel)
        .register(LlvmPreoptLevel { shared: codegen.clone() })
        .register(LlvmOptLevel { shared: codegen.clone() })
        .register(AsmLevel { shared: codegen.clone() })
        .register(ObjLevel { shared: codegen.clone() })
        .register(BinLevel { shared: codegen })
        .register(SymTableLevel { shared: binary.clone() })
        .register(SymDemangledLevel { shared: binary.clone() })
        .register(SymSectionsLevel { shared: binary.clone() })
        .register(SymRelocsLevel { shared: binary.clone() })
        .register(DwarfLevel { shared: binary.clone() })
        .register(LinkMapLevel { shared: binary })
        .register(TypeUsageLevel { shared: ast.clone() })
        .register(TypeTraitImplLevel { shared: rustdoc.clone() })
        .register(TypeCoherenceLevel { shared: rustdoc.clone() })
        .register(RustdocLevel { shared: rustdoc })
        .register(ClippyLevel)
        .register(TestsLevel)
        .register(DoctestsLevel)
        .register(GraphModLevel { shared: graphs.clone() })
        .register(GraphCallStaticLevel { shared: graphs.clone() })
        .register(GraphCfgLevel { shared: graphs.clone(), ast })
        .register(GraphTypeUsageLevel { shared: graphs.clone() })
        .register(GraphSccsLevel { shared: graphs.clone() })
        .register(GraphCommunitiesLevel { shared: graphs.clone() })
        .register(GraphPowerLawLevel { shared: graphs.clone() })
        .register(GraphSelfSimLevel { shared: graphs })
        .register(AstResolvedLevel { shared: ir.clone() })
        .register(HirLevel { shared: ir.clone() })
        .register(ThirLevel { shared: ir.clone() })
        .register(MirPreoptLevel { shared: ir.clone() })
        .register(MirOptLevel { shared: ir.clone() })
        .register(BorrowckLevel { shared: ir.clone() })
        .register(TypeMonoLevel { shared: ir })
        .register(GraphCallMonoLevel { shared: llvm_model.clone() })
        .register(GraphDataflowLevel { shared: llvm_model });
}
