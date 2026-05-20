//! Stable mirror types for rustc-internal IRs (spec §2.3, §2.4, plan Stage 11).
//!
//! The nightly sub-binary (`cstat-extract-rustc`) walks `rustc_driver`'s
//! `TyCtxt` and translates the bits we measure into the plain serde-derive
//! types defined here. The orchestrator and every other stable crate
//! depend only on this crate — never on `rustc_*`. That keeps the
//! isolation boundary honest: when rustc churns, the translator updates;
//! these mirror types stay put.
//!
//! Design rules:
//! - **No rustc deps.** Anything in this crate must build on stable Rust.
//! - **Only fields a downstream metric consumes.** We are not mirroring
//!   rustc faithfully; we are recording the inputs to specific Level
//!   impls back on the stable side.
//! - **String discriminants for IR enums.** Expression-kind histograms,
//!   MIR statement kinds, MIR terminators, mono-item kinds — all encoded
//!   as strings. A future rustc that adds a new variant lands as a new
//!   string in the histogram; old extractors keep parsing.
//! - **`#[serde(default)]` on collection fields.** A nightly extractor
//!   that doesn't emit a section yet should produce a dump older readers
//!   accept, not a parse error.

pub mod borrowck;
pub mod hir;
pub mod mir;
pub mod monoitems;
pub mod thir;
pub mod toolchain;

pub use borrowck::{BorrowckBody, BorrowckDump};
pub use hir::{HirBinding, HirDump, HirImport, HirItem};
pub use mir::{MirBasicBlock, MirBody, MirDump};
pub use monoitems::{GenericInstantiationGroup, MonoItem, MonoItemsDump};
pub use thir::{ThirBody, ThirDump};
pub use toolchain::ToolchainStamp;

use serde::{Deserialize, Serialize};

/// Version stamp for the `cstat-ir` JSON dump format. Bumps on any
/// breaking change to the mirror types. Independent of the schema
/// version in `cstat-schema` — this is the wire format between the
/// nightly sub-binary and the stable orchestrator, not the user-facing
/// envelope schema.
pub const IR_FORMAT_VERSION: &str = "0.1.0";

/// Top-level dump emitted by `cstat-extract-rustc`. Each IR section is
/// optional so the sub-binary can be invoked for a subset of levels
/// without padding the output with empty stubs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CstatIrDump {
    pub ir_format_version: String,
    pub toolchain: ToolchainStamp,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hir: Option<HirDump>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub thir: Option<ThirDump>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mir_preopt: Option<MirDump>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mir_opt: Option<MirDump>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub borrowck: Option<BorrowckDump>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mono_items: Option<MonoItemsDump>,
}

impl CstatIrDump {
    /// Empty dump stamped with the current IR format version and the
    /// given toolchain. All IR sections start `None`; translators fill
    /// in only what they emit.
    pub fn empty(toolchain: ToolchainStamp) -> Self {
        Self {
            ir_format_version: IR_FORMAT_VERSION.to_string(),
            toolchain,
            hir: None,
            thir: None,
            mir_preopt: None,
            mir_opt: None,
            borrowck: None,
            mono_items: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_dump() -> CstatIrDump {
        let mut dump = CstatIrDump::empty(ToolchainStamp {
            channel: "nightly".into(),
            version: "1.97.0-nightly".into(),
            host: "x86_64-unknown-linux-gnu".into(),
        });
        dump.hir = Some(hir::tests::sample_hir());
        dump.thir = Some(thir::tests::sample_thir());
        dump.mir_preopt = Some(mir::tests::sample_mir("pre"));
        dump.mir_opt = Some(mir::tests::sample_mir("opt"));
        dump.borrowck = Some(borrowck::tests::sample_borrowck());
        dump.mono_items = Some(monoitems::tests::sample_mono());
        dump
    }

    #[test]
    fn full_dump_roundtrips_through_json_losslessly() {
        let original = sample_dump();
        let json = serde_json::to_string(&original).unwrap();
        let decoded: CstatIrDump = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn empty_dump_omits_absent_sections() {
        let dump = CstatIrDump::empty(ToolchainStamp {
            channel: "nightly".into(),
            version: "1.97.0-nightly".into(),
            host: "x86_64-unknown-linux-gnu".into(),
        });
        let json = serde_json::to_string(&dump).unwrap();
        // Absent sections must not appear in the JSON — omission is the
        // contract for "this nightly extractor didn't emit this layer".
        for missing in [
            "\"hir\"",
            "\"thir\"",
            "\"mir_preopt\"",
            "\"mir_opt\"",
            "\"borrowck\"",
            "\"mono_items\"",
        ] {
            assert!(
                !json.contains(missing),
                "expected `{missing}` to be absent in empty dump JSON, got: {json}"
            );
        }
        // Round trip stays empty.
        let decoded: CstatIrDump = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, dump);
    }

    #[test]
    fn dump_tolerates_missing_optional_sections_on_read() {
        // A future or older nightly might emit only the toolchain stamp
        // and version. The reader must accept that.
        let minimal = r#"{
            "ir_format_version": "0.1.0",
            "toolchain": {"channel":"nightly","version":"x","host":"y"}
        }"#;
        let decoded: CstatIrDump = serde_json::from_str(minimal).unwrap();
        assert_eq!(decoded.ir_format_version, "0.1.0");
        assert!(decoded.hir.is_none());
        assert!(decoded.mir_preopt.is_none());
    }
}
