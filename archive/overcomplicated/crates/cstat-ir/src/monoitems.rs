//! Mono-item mirror types (spec §2.4 #18).
//!
//! `tcx.collect_and_partition_mono_items` yields the set of (def, type
//! substitution) pairs the codegen will emit. We mirror that as a flat
//! list plus a per-source-def rollup so level 18 can read either form
//! without scanning twice.

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonoItemsDump {
    /// One entry per (source def, type substitution) pair.
    #[serde(default)]
    pub items: Vec<MonoItem>,
    /// Rollup: how many instantiations each generic source-def
    /// received. Derivable from `items` but explicit so consumers
    /// don't all reimplement it.
    #[serde(default)]
    pub generic_instantiation_groups: Vec<GenericInstantiationGroup>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonoItem {
    /// Generic source-def path (no substitutions), e.g.
    /// `core::option::Option::<T>::map`.
    pub def_path: String,
    /// String discriminant: "fn" | "method" | "static" | "drop_glue" |
    /// "clone_shim" | "vtable_shim" | "reify_shim" | "fn_ptr_shim" |
    /// "intrinsic" | ... — whatever rustc names it on this nightly.
    pub kind: String,
    /// Demangled-style substitution string, e.g.
    /// `core::option::Option::<i32>::map::<u8>`. The translator picks
    /// the format; the stable-side metric extractor only counts.
    pub instantiation: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenericInstantiationGroup {
    pub source_def_path: String,
    pub instantiation_count: u32,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_mono() -> MonoItemsDump {
        MonoItemsDump {
            items: vec![
                MonoItem {
                    def_path: "example::classify".into(),
                    kind: "fn".into(),
                    instantiation: "example::classify".into(),
                },
                MonoItem {
                    def_path: "example::pair".into(),
                    kind: "fn".into(),
                    instantiation: "example::pair::<i32, u8>".into(),
                },
                MonoItem {
                    def_path: "example::pair".into(),
                    kind: "fn".into(),
                    instantiation: "example::pair::<String, bool>".into(),
                },
            ],
            generic_instantiation_groups: vec![
                GenericInstantiationGroup {
                    source_def_path: "example::classify".into(),
                    instantiation_count: 1,
                },
                GenericInstantiationGroup {
                    source_def_path: "example::pair".into(),
                    instantiation_count: 2,
                },
            ],
        }
    }

    #[test]
    fn mono_items_dump_roundtrips_through_json() {
        let original = sample_mono();
        let json = serde_json::to_string(&original).unwrap();
        let decoded: MonoItemsDump = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn mono_items_dump_accepts_minimal_input() {
        let decoded: MonoItemsDump = serde_json::from_str("{}").unwrap();
        assert_eq!(decoded, MonoItemsDump::default());
    }

    #[test]
    fn unknown_kind_strings_are_preserved() {
        // Future rustc may invent a kind we don't recognise; it must
        // still round-trip cleanly through the mirror.
        let item = MonoItem {
            def_path: "example::weird".into(),
            kind: "novel_shim_kind_42".into(),
            instantiation: "example::weird".into(),
        };
        let json = serde_json::to_string(&item).unwrap();
        let decoded: MonoItem = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, item);
    }
}
