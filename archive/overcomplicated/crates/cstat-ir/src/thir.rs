//! THIR mirror types (spec §2.3 #12, plus the name-resolved variant of
//! level 16).
//!
//! THIR is HIR with full type information attached. The translator
//! rolls that up to per-body scalars rather than re-emitting every
//! expression's type — that's what the downstream metrics consume.

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThirDump {
    #[serde(default)]
    pub bodies: Vec<ThirBody>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThirBody {
    pub def_path: String,
    /// Total expressions in the body. Denominator for the densities
    /// below; emitted explicitly rather than computed downstream so
    /// extractors don't have to re-derive it.
    pub expr_count: u32,
    /// Distinct fully-resolved types observed across expressions and
    /// pattern bindings in this body.
    pub distinct_type_count: u32,
    /// Sum of generic-arg list lengths across this body's expressions.
    /// E.g. a `Vec<HashMap<K, V>>` contributes 1 (Vec's 1 arg) + 2
    /// (HashMap's 2 args) at each use site. Density = this / expr_count.
    pub generic_arg_total: u32,
    /// Expressions whose static type is a `dyn Trait`.
    pub trait_object_count: u32,
    /// Expressions whose static type contains at least one generic
    /// parameter or type argument. Density = this / expr_count.
    pub generic_expr_count: u32,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_thir() -> ThirDump {
        ThirDump {
            bodies: vec![
                ThirBody {
                    def_path: "example::classify".into(),
                    expr_count: 12,
                    distinct_type_count: 4,
                    generic_arg_total: 0,
                    trait_object_count: 0,
                    generic_expr_count: 0,
                },
                ThirBody {
                    def_path: "example::generic_join".into(),
                    expr_count: 20,
                    distinct_type_count: 7,
                    generic_arg_total: 9,
                    trait_object_count: 1,
                    generic_expr_count: 6,
                },
            ],
        }
    }

    #[test]
    fn thir_dump_roundtrips_through_json() {
        let original = sample_thir();
        let json = serde_json::to_string(&original).unwrap();
        let decoded: ThirDump = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn thir_dump_accepts_minimal_input() {
        let decoded: ThirDump = serde_json::from_str("{}").unwrap();
        assert_eq!(decoded, ThirDump::default());
    }
}
