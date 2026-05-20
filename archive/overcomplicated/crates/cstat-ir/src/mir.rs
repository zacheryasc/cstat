//! MIR mirror types (spec §2.3 #13 and #14).
//!
//! One [`MirBody`] per function. The same struct mirrors both pre-opt
//! MIR (level 13) and post-opt MIR (level 14); the top-level
//! [`crate::CstatIrDump`] holds two snapshots so downstream extractors
//! can take their delta.
//!
//! Statement kinds and terminators are plain strings (`"Assign"`,
//! `"StorageLive"`, `"Return"`, `"SwitchInt"`, `"Drop"`, ...). The
//! string set is whatever the translator emits — a new rustc release
//! that adds a variant lands as a new string, never a parse error.

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct MirDump {
    #[serde(default)]
    pub bodies: Vec<MirBody>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct MirBody {
    /// Crate-qualified def path of the function this body belongs to.
    pub def_path: String,
    /// Number of arg locals (locals `_1..=_arg_count`).
    pub arg_count: u32,
    /// Total local count including return slot, args, and temporaries.
    pub local_count: u32,
    #[serde(default)]
    pub basic_blocks: Vec<MirBasicBlock>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct MirBasicBlock {
    pub id: u32,
    /// String discriminants for statements in source order. The
    /// terminator is recorded separately under [`Self::terminator`].
    #[serde(default)]
    pub statement_kinds: Vec<String>,
    /// String discriminant for the terminator (`"Return"`, `"Goto"`,
    /// `"SwitchInt"`, `"Call"`, `"Drop"`, `"Unreachable"`, ...).
    pub terminator: String,
    #[serde(default)]
    pub predecessors: Vec<u32>,
    #[serde(default)]
    pub successors: Vec<u32>,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_mir(label: &str) -> MirDump {
        MirDump {
            bodies: vec![MirBody {
                def_path: format!("example::classify::{label}"),
                arg_count: 1,
                local_count: 5,
                basic_blocks: vec![
                    MirBasicBlock {
                        id: 0,
                        statement_kinds: vec!["StorageLive".into(), "Assign".into()],
                        terminator: "SwitchInt".into(),
                        predecessors: vec![],
                        successors: vec![1, 2, 3],
                    },
                    MirBasicBlock {
                        id: 1,
                        statement_kinds: vec!["Assign".into()],
                        terminator: "Goto".into(),
                        predecessors: vec![0],
                        successors: vec![4],
                    },
                    MirBasicBlock {
                        id: 2,
                        statement_kinds: vec!["Assign".into()],
                        terminator: "Goto".into(),
                        predecessors: vec![0],
                        successors: vec![4],
                    },
                    MirBasicBlock {
                        id: 3,
                        statement_kinds: vec!["Assign".into()],
                        terminator: "Goto".into(),
                        predecessors: vec![0],
                        successors: vec![4],
                    },
                    MirBasicBlock {
                        id: 4,
                        statement_kinds: vec![],
                        terminator: "Return".into(),
                        predecessors: vec![1, 2, 3],
                        successors: vec![],
                    },
                ],
            }],
        }
    }

    #[test]
    fn mir_dump_roundtrips_through_json() {
        let original = sample_mir("pre");
        let json = serde_json::to_string(&original).unwrap();
        let decoded: MirDump = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn mir_dump_accepts_minimal_input() {
        let decoded: MirDump = serde_json::from_str("{}").unwrap();
        assert_eq!(decoded, MirDump::default());
    }

    #[test]
    fn empty_body_has_no_blocks_but_still_roundtrips() {
        let body = MirBody {
            def_path: "x::y".into(),
            arg_count: 0,
            local_count: 1,
            basic_blocks: vec![],
        };
        let json = serde_json::to_string(&body).unwrap();
        let decoded: MirBody = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, body);
    }
}
