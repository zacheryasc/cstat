//! Output envelope and identity types for cstat (Milestone 1 §5–§7).
//!
//! These types ARE the contract. Every downstream consumer reads this
//! shape. Schema version starts at 0.1.0; bump rules per spec §10.

pub mod ids;
pub mod envelope;

pub use envelope::{
    CostClass, EntityRecord, LevelReport, LevelStatus, OracleOutput, Provenance, Toolchain, Vector,
};
pub use ids::{AstFuncId, FileId, IdSet, ModuleId, MonoId, SymbolId};

/// Current schema version, per spec §10.
pub const SCHEMA_VERSION: &str = "0.1.0";
