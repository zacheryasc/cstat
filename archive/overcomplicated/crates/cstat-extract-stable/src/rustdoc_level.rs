//! Spec §2.9 level 46 — rustdoc JSON inventory.
//!
//! Shares the [`SharedRustdoc`] cache with §2.4 levels 17 and 19, so
//! `cargo rustdoc` fires at most once per `cstat extract` run no matter
//! how many rustdoc-derived levels are registered. Reports per-kind
//! item counts and the documented-item ratio.

use std::collections::BTreeMap;

use cstat_orchestrate::{Context, CostClass, Level, LevelError, LevelOutput};

use crate::rustdoc_cache::{IndexItem, SharedRustdoc};

pub(crate) struct RustdocLevel {
    pub shared: SharedRustdoc,
}

impl Level for RustdocLevel {
    fn id(&self) -> &'static str { "rustdoc" }
    fn cost_class(&self) -> CostClass { CostClass::B }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let doc = self.shared.get(ctx)?;
        let stats = ItemStats::collect(doc.data.index.values());

        let mut vector = vec![
            ("rustdoc.format_version".into(), doc.data.format_version as f64),
            ("rustdoc.index_size".into(), stats.total as f64),
            ("rustdoc.local_item_count".into(), stats.local_count as f64),
            ("rustdoc.foreign_item_count".into(), stats.foreign_count as f64),
            ("rustdoc.documented_count".into(), stats.documented as f64),
            (
                "rustdoc.documented_ratio".into(),
                ratio(stats.documented, stats.local_count),
            ),
            ("rustdoc.public_item_count".into(), stats.public as f64),
            ("rustdoc.distinct_kind_count".into(), stats.kinds.len() as f64),
        ];

        // Per-kind counts under `rustdoc.kind.<name>` for the kinds we
        // actually observed. `BTreeMap` keeps the keys sorted so the
        // vector is deterministic across runs.
        for (kind, count) in &stats.kinds {
            vector.push((format!("rustdoc.kind.{kind}"), *count as f64));
        }

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

#[derive(Default)]
struct ItemStats {
    total: u64,
    local_count: u64,
    foreign_count: u64,
    documented: u64,
    public: u64,
    kinds: BTreeMap<String, u64>,
}

impl ItemStats {
    fn collect<'a, I: IntoIterator<Item = &'a IndexItem>>(items: I) -> Self {
        let mut stats = ItemStats::default();
        for item in items {
            stats.total += 1;
            if item.crate_id == 0 {
                stats.local_count += 1;
            } else {
                stats.foreign_count += 1;
            }

            // Item kind comes from the single key of the `inner` object
            // — rustdoc tags variants by key (`"function"`, `"struct"`,
            // `"impl"`, …). Anonymous shapes (impl blocks etc.) bucket
            // under their key just the same.
            if let Some(kind) = kind_of(&item.inner) {
                *stats.kinds.entry(kind).or_insert(0) += 1;
            }

            if item
                .docs
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
            {
                stats.documented += 1;
            }
            if is_public_visibility(&item.visibility) {
                stats.public += 1;
            }
        }
        stats
    }
}

/// Across rustdoc-JSON versions, `visibility` has been encoded as either
/// the string `"public"` or as an object like `{"restricted": {...}}`.
/// Treat the string form as the source of truth for the `public` count;
/// object forms are always restricted in some way.
fn is_public_visibility(value: &serde_json::Value) -> bool {
    matches!(value.as_str(), Some("public"))
}

fn kind_of(inner: &serde_json::Value) -> Option<String> {
    let obj = inner.as_object()?;
    // Rustdoc-JSON wraps each variant under a single object key. Find
    // the first string-keyed entry.
    obj.keys().next().cloned()
}

fn ratio(num: u64, denom: u64) -> f64 {
    if denom == 0 { 0.0 } else { num as f64 / denom as f64 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(crate_id: u32, inner: serde_json::Value) -> IndexItem {
        IndexItem {
            name: None,
            crate_id,
            docs: None,
            visibility: serde_json::Value::Null,
            inner,
        }
    }

    #[test]
    fn collect_splits_local_vs_foreign_and_counts_kinds() {
        let items = vec![
            item(0, json!({"function": {}})),
            item(0, json!({"struct": {}})),
            item(0, json!({"function": {}})),
            item(1, json!({"trait": {}})),
        ];
        let s = ItemStats::collect(items.iter());
        assert_eq!(s.total, 4);
        assert_eq!(s.local_count, 3);
        assert_eq!(s.foreign_count, 1);
        assert_eq!(s.kinds.get("function").copied(), Some(2));
        assert_eq!(s.kinds.get("struct").copied(), Some(1));
        assert_eq!(s.kinds.get("trait").copied(), Some(1));
    }

    #[test]
    fn ratio_handles_zero_denominator() {
        assert_eq!(ratio(0, 0), 0.0);
        assert_eq!(ratio(3, 6), 0.5);
    }
}
