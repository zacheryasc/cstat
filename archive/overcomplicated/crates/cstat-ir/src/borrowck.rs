//! Borrow-check mirror types (spec §2.3 #15).
//!
//! Region/lifetime constraints and NLL conflict counts rolled up
//! per-body. The full constraint graph is much larger than v1 needs;
//! per-body scalars are what level 15 emits.

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct BorrowckDump {
    #[serde(default)]
    pub bodies: Vec<BorrowckBody>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BorrowckBody {
    pub def_path: String,
    /// Total region variables introduced for this body.
    pub region_count: u32,
    /// `'a: 'b` outlives obligations in the region constraint graph.
    pub outlives_constraint_count: u32,
    /// Member constraints (opaque-type / hidden-region inference edges).
    pub member_constraint_count: u32,
    /// Conflicts the NLL solver had to resolve (i.e. constraint
    /// propagation rounds that changed at least one region's value).
    pub nll_conflict_count: u32,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_borrowck() -> BorrowckDump {
        BorrowckDump {
            bodies: vec![BorrowckBody {
                def_path: "example::classify".into(),
                region_count: 3,
                outlives_constraint_count: 4,
                member_constraint_count: 0,
                nll_conflict_count: 0,
            }],
        }
    }

    #[test]
    fn borrowck_dump_roundtrips_through_json() {
        let original = sample_borrowck();
        let json = serde_json::to_string(&original).unwrap();
        let decoded: BorrowckDump = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn borrowck_dump_accepts_minimal_input() {
        let decoded: BorrowckDump = serde_json::from_str("{}").unwrap();
        assert_eq!(decoded, BorrowckDump::default());
    }
}
