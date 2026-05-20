//! HIR mirror types (spec §2.2 #10 + §2.3 #11).
//!
//! The HIR is the highest-level IR rustc carries internally: desugared
//! control flow with source attribution preserved. The translator emits
//! one [`HirItem`] per top-level item (fn / impl method / static / const
//! / trait / impl block / module / use), plus the [`HirImport`] and
//! [`HirBinding`] streams that level 10 reads to reconstruct the import
//! graph and shadowing structure.
//!
//! Expression kinds are encoded as plain strings (`"if"`, `"match"`,
//! `"loop"`, `"await"`, `"?"`, `"method_call"`, ...) so rustc can grow a
//! new variant without breaking the wire format. The translator picks
//! the string; the stable-side metric extractor groups them.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirDump {
    #[serde(default)]
    pub items: Vec<HirItem>,
    #[serde(default)]
    pub imports: Vec<HirImport>,
    #[serde(default)]
    pub bindings: Vec<HirBinding>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirItem {
    /// Crate-qualified def path, e.g. `mycrate::module::function`.
    pub def_path: String,
    /// Item kind as a string discriminant: "fn", "method", "struct",
    /// "enum", "trait", "impl", "static", "const", "type_alias", ...
    pub kind: String,
    /// Number of branching expressions (if/match/loop/while/&&/||).
    pub branch_count: u32,
    /// Total expressions visited in the body (0 for non-body items).
    pub expr_count: u32,
    /// Histogram of expression-kind strings. Sum may exceed
    /// `expr_count` in future rustc revisions if the translator emits
    /// overlapping classifications — consumers should treat this as a
    /// frequency table per kind, not a strict partition.
    #[serde(default)]
    pub expr_kinds: BTreeMap<String, u32>,
}

/// Edge in the import graph rebuilt from `use` declarations after name
/// resolution. `from_module` is the module that wrote the `use`;
/// `resolved_def_path` is the canonical def path the name resolves to.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirImport {
    pub from_module: String,
    pub resolved_def_path: String,
    /// "single" | "glob" | "renamed"
    pub kind: String,
}

/// One name binding introduced in a body. Multiple bindings of the same
/// name in the same scope appear with increasing `shadow_index`; the
/// first occurrence is 0.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirBinding {
    pub name: String,
    pub scope_def_path: String,
    pub shadow_index: u32,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_hir() -> HirDump {
        let mut expr_kinds = BTreeMap::new();
        expr_kinds.insert("if".to_string(), 2);
        expr_kinds.insert("method_call".to_string(), 5);
        expr_kinds.insert("?".to_string(), 1);
        HirDump {
            items: vec![HirItem {
                def_path: "example::classify".into(),
                kind: "fn".into(),
                branch_count: 3,
                expr_count: 12,
                expr_kinds,
            }],
            imports: vec![HirImport {
                from_module: "example".into(),
                resolved_def_path: "std::fmt::Display".into(),
                kind: "single".into(),
            }],
            bindings: vec![
                HirBinding {
                    name: "x".into(),
                    scope_def_path: "example::classify".into(),
                    shadow_index: 0,
                },
                HirBinding {
                    name: "x".into(),
                    scope_def_path: "example::classify".into(),
                    shadow_index: 1,
                },
            ],
        }
    }

    #[test]
    fn hir_dump_roundtrips_through_json() {
        let original = sample_hir();
        let json = serde_json::to_string(&original).unwrap();
        let decoded: HirDump = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn hir_dump_accepts_minimal_input() {
        // Empty object is enough — collections default to empty.
        let decoded: HirDump = serde_json::from_str("{}").unwrap();
        assert_eq!(decoded, HirDump::default());
    }

    #[test]
    fn unknown_expr_kind_strings_are_preserved() {
        // The whole point of string discriminants: a future rustc could
        // emit a kind we don't recognise and it must round-trip cleanly.
        let mut expr_kinds = BTreeMap::new();
        expr_kinds.insert("become".to_string(), 1);
        expr_kinds.insert("yeet".to_string(), 2);
        let item = HirItem {
            def_path: "x::y".into(),
            kind: "fn".into(),
            branch_count: 0,
            expr_count: 3,
            expr_kinds,
        };
        let json = serde_json::to_string(&item).unwrap();
        let decoded: HirItem = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, item);
    }
}
