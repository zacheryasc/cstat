//! Output envelope per spec §5 and §7.
//!
//! BTreeMap is used throughout so iteration order is deterministic for
//! snapshot tests and byte-identical re-runs.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ids::IdSet;

/// Flat namespace from dotted keys to f64 scalars (spec §5).
///
/// Newtype around a BTreeMap for deterministic iteration. Counts are f64
/// for uniformity; booleans become 0.0/1.0.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Vector(pub BTreeMap<String, f64>);

impl Vector {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn insert(&mut self, key: impl Into<String>, value: f64) -> Option<f64> {
        self.0.insert(key.into(), value)
    }

    pub fn get(&self, key: &str) -> Option<f64> {
        self.0.get(key).copied()
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> std::collections::btree_map::Iter<'_, String, f64> {
        self.0.iter()
    }
}

/// Per-entity record (spec §6).
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityRecord {
    pub ids: IdSet,
    #[serde(default)]
    pub metrics: BTreeMap<String, f64>,
}

/// Level extraction status per spec §7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LevelStatus {
    Ok,
    Skipped,
    Failed,
}

/// Per-level report in the output envelope (spec §7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelReport {
    pub status: LevelStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
    pub recompute_cost_class: CostClass,
    pub deterministic: bool,
    pub extraction_seconds: f64,
}

/// Cost class per spec §2 preamble: F/L/B/C.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostClass {
    /// Free during a build we already run.
    F,
    /// Local pass over already-extracted artifacts.
    L,
    /// Requires a build or rebuild.
    B,
    /// Requires a codegen step beyond a normal check build.
    C,
}

/// Toolchain stamp per spec §7.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Toolchain {
    pub channel: String,
    pub version: String,
    pub host: String,
}

/// For each dotted key, the list of level ids that contributed (spec §7).
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Provenance(pub BTreeMap<String, Vec<String>>);

impl Provenance {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn record(&mut self, key: impl Into<String>, level: impl Into<String>) {
        self.0.entry(key.into()).or_default().push(level.into());
    }
}

/// The output envelope (spec §7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OracleOutput {
    pub schema_version: String,
    pub cstat_version: String,
    pub toolchain: Toolchain,
    pub config_digest: String,
    pub wall_clock_seconds: f64,
    pub levels: BTreeMap<String, LevelReport>,
    pub vector: Vector,
    pub entities: BTreeMap<String, Vec<EntityRecord>>,
    pub provenance: Provenance,
}

impl OracleOutput {
    /// Construct an empty envelope stamped with the current schema version.
    pub fn empty(cstat_version: impl Into<String>, toolchain: Toolchain) -> Self {
        Self {
            schema_version: crate::SCHEMA_VERSION.to_string(),
            cstat_version: cstat_version.into(),
            toolchain,
            config_digest: String::new(),
            wall_clock_seconds: 0.0,
            levels: BTreeMap::new(),
            vector: Vector::new(),
            entities: BTreeMap::new(),
            provenance: Provenance::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::*;

    fn sample_envelope() -> OracleOutput {
        let mut out = OracleOutput::empty(
            "0.1.0",
            Toolchain {
                channel: "stable".into(),
                version: "1.95.0".into(),
                host: "x86_64-unknown-linux-gnu".into(),
            },
        );
        out.config_digest = "abc123".into();
        out.wall_clock_seconds = 1.25;
        out.levels.insert(
            "fs".into(),
            LevelReport {
                status: LevelStatus::Ok,
                reason: None,
                recompute_cost_class: CostClass::F,
                deterministic: true,
                extraction_seconds: 0.01,
            },
        );
        out.levels.insert(
            "mir".into(),
            LevelReport {
                status: LevelStatus::Skipped,
                reason: Some("nightly toolchain unavailable".into()),
                recompute_cost_class: CostClass::B,
                deterministic: true,
                extraction_seconds: 0.0,
            },
        );
        out.vector.insert("fs.global.file_count", 1.0);
        out.vector.insert("fs.global.total_bytes", 184.0);
        out.entities.insert(
            "ast.func".into(),
            vec![EntityRecord {
                ids: IdSet {
                    file: Some(FileId::new("src/lib.rs")),
                    ast_func: Some(AstFuncId {
                        file: FileId::new("src/lib.rs"),
                        fq_path: "example::classify".into(),
                        item_kind: "fn".into(),
                        ast_node_hash: "h2".into(),
                    }),
                    symbol: Some(SymbolId::new("_ZN7example8classify17h2")),
                    ..Default::default()
                },
                metrics: [("loc".to_string(), 7.0), ("cyclomatic".to_string(), 3.0)]
                    .into_iter()
                    .collect(),
            }],
        );
        out.provenance.record("fs.global.file_count", "fs");
        out
    }

    #[test]
    fn envelope_roundtrips_through_json_losslessly() {
        let original = sample_envelope();
        let json = serde_json::to_string(&original).unwrap();
        let decoded: OracleOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);

        // Also: JSON → Value → JSON is byte-identical (deterministic key order).
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let rejson = serde_json::to_string(&v).unwrap();
        let v2: serde_json::Value = serde_json::from_str(&rejson).unwrap();
        assert_eq!(v, v2);
    }

    #[test]
    fn id_types_serialize_as_documented() {
        let ids = IdSet {
            file: Some(FileId::new("src/lib.rs")),
            module: Some(ModuleId::new("example::inner")),
            ast_func: Some(AstFuncId {
                file: FileId::new("src/lib.rs"),
                fq_path: "example::classify".into(),
                item_kind: "fn".into(),
                ast_node_hash: "h2".into(),
            }),
            symbol: Some(SymbolId::new("_ZN7example8classify17h2")),
            mono: Some(MonoId::new("example::classify::<i32>")),
        };
        let json = serde_json::to_string(&ids).unwrap();
        // Spec §13 shows IdSet keys as "AstFuncId", "SymbolId", etc.
        assert!(json.contains("\"AstFuncId\""));
        assert!(json.contains("\"SymbolId\""));
        assert!(json.contains("\"ModuleId\""));
        assert!(json.contains("\"FileId\""));
        assert!(json.contains("\"MonoId\""));

        let decoded: IdSet = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, ids);
    }

    #[test]
    fn empty_idset_omits_fields() {
        let ids = IdSet::default();
        let json = serde_json::to_string(&ids).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn vector_iteration_is_deterministic() {
        let mut v = Vector::new();
        v.insert("z.last", 3.0);
        v.insert("a.first", 1.0);
        v.insert("m.middle", 2.0);
        let keys: Vec<&String> = v.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec!["a.first", "m.middle", "z.last"]);
    }
}
