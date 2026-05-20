//! Spec §2.4 levels 16 (type-usage graph), 17 (trait impl graph), and
//! 19 (coherence / orphan structure).
//!
//! Level 16 is the AST-approximation: per-function distinct-type counts
//! derived from `syn::Type` walking. The name-resolved variant lives in
//! the nightly path (Stage 13).
//!
//! Levels 17 and 19 both consume rustdoc JSON (nightly-only, treated as
//! a subprocess so it doesn't compromise the rustc-isolation boundary).
//! They share one [`SharedRustdoc`] cache: the subprocess runs at most
//! once per `cstat extract` invocation regardless of how many rustdoc-
//! derived levels are registered. When nightly isn't installed both
//! levels skip with the same reason.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use cstat_orchestrate::{Context, CostClass, EntityRecord, Level, LevelError, LevelOutput};
use cstat_schema::{AstFuncId, FileId, IdSet};

use crate::ast_cache::SharedAst;
use crate::rustdoc_cache::{IndexItem, RustdocJson, SharedRustdoc};

// ───── Level 16 (AST approximation): per-function type-usage counts ─────

pub(crate) struct TypeUsageLevel {
    pub shared: SharedAst,
}

impl Level for TypeUsageLevel {
    fn id(&self) -> &'static str { "type.usage" }
    fn cost_class(&self) -> CostClass { CostClass::L }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let ast = self.shared.get(&ctx.project_root);

        let mut per_func_distinct: Vec<u64> = Vec::new();
        let mut per_func_param_distinct: Vec<u64> = Vec::new();
        let mut per_func_return_distinct: Vec<u64> = Vec::new();
        let mut entity_records: Vec<EntityRecord> = Vec::new();
        let mut all_distinct: BTreeSet<String> = BTreeSet::new();
        let mut all_primitive: BTreeSet<String> = BTreeSet::new();

        for fs in ast.iter_functions() {
            let usage = &fs.type_usage;
            per_func_distinct.push(usage.distinct.len() as u64);
            per_func_param_distinct.push(usage.param_distinct.len() as u64);
            per_func_return_distinct.push(usage.return_distinct.len() as u64);
            all_distinct.extend(usage.distinct.iter().cloned());
            all_primitive.extend(usage.primitives.iter().cloned());

            let mut metrics = BTreeMap::new();
            metrics.insert("distinct_types".into(), usage.distinct.len() as f64);
            metrics.insert("param_distinct_types".into(), usage.param_distinct.len() as f64);
            metrics.insert("return_distinct_types".into(), usage.return_distinct.len() as f64);
            metrics.insert("primitive_types".into(), usage.primitives.len() as f64);
            let file = FileId::new(posix(&fs.file_path));
            entity_records.push(EntityRecord {
                ids: IdSet {
                    file: Some(file.clone()),
                    ast_func: Some(AstFuncId {
                        file,
                        fq_path: fs.fq_path.clone(),
                        item_kind: "fn".into(),
                        ast_node_hash: fs.node_hash.clone(),
                    }),
                    ..Default::default()
                },
                metrics,
            });
        }

        let function_count = per_func_distinct.len() as f64;
        let (max_d, mean_d, sum_d) = summarize(&per_func_distinct);
        let (max_p, mean_p, _sum_p) = summarize(&per_func_param_distinct);
        let (max_r, mean_r, _sum_r) = summarize(&per_func_return_distinct);

        let vector = vec![
            ("type.usage.function_count".into(), function_count),
            ("type.usage.distinct_types_overall".into(), all_distinct.len() as f64),
            ("type.usage.primitive_types_overall".into(), all_primitive.len() as f64),
            ("type.usage.func.distinct_types.max".into(), max_d),
            ("type.usage.func.distinct_types.mean".into(), mean_d),
            ("type.usage.func.distinct_types.sum".into(), sum_d),
            ("type.usage.func.param_distinct_types.max".into(), max_p),
            ("type.usage.func.param_distinct_types.mean".into(), mean_p),
            ("type.usage.func.return_distinct_types.max".into(), max_r),
            ("type.usage.func.return_distinct_types.mean".into(), mean_r),
        ];

        let mut entities: BTreeMap<String, Vec<EntityRecord>> = BTreeMap::new();
        if !entity_records.is_empty() {
            entities.insert("type.usage_func".into(), entity_records);
        }

        Ok(LevelOutput { vector, entities })
    }
}

fn summarize(values: &[u64]) -> (f64, f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let sum: u64 = values.iter().sum();
    let max = *values.iter().max().unwrap();
    let mean = sum as f64 / values.len() as f64;
    (max as f64, mean, sum as f64)
}

// ───── Levels 17 + 19: rustdoc-JSON-derived trait / coherence views ─────

pub(crate) struct TypeTraitImplLevel {
    pub shared: SharedRustdoc,
}

impl Level for TypeTraitImplLevel {
    fn id(&self) -> &'static str { "type.trait_impl" }
    fn cost_class(&self) -> CostClass { CostClass::B }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let doc = self.shared.get(ctx)?;
        let view = ImplView::collect(&doc.data);

        let vector = vec![
            ("type.trait_impl.format_version".into(), doc.data.format_version as f64),
            ("type.trait_impl.trait_count".into(), view.local_trait_count as f64),
            ("type.trait_impl.impl_count".into(), view.impl_count as f64),
            ("type.trait_impl.inherent_impl_count".into(), view.inherent_impl_count as f64),
            ("type.trait_impl.trait_impl_count".into(), view.trait_impl_count as f64),
            ("type.trait_impl.blanket_impl_count".into(), view.blanket_impl_count as f64),
            ("type.trait_impl.synthetic_impl_count".into(), view.synthetic_impl_count as f64),
            ("type.trait_impl.negative_impl_count".into(), view.negative_impl_count as f64),
            ("type.trait_impl.distinct_impl_traits".into(), view.distinct_impl_traits as f64),
            ("type.trait_impl.distinct_impl_types".into(), view.distinct_impl_types as f64),
        ];
        Ok(LevelOutput { vector, ..Default::default() })
    }
}

pub(crate) struct TypeCoherenceLevel {
    pub shared: SharedRustdoc,
}

impl Level for TypeCoherenceLevel {
    fn id(&self) -> &'static str { "type.coherence" }
    fn cost_class(&self) -> CostClass { CostClass::B }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let doc = self.shared.get(ctx)?;
        let view = ImplView::collect(&doc.data);

        // Orphan-rule split: which quadrant does each trait impl land in?
        // Local trait + local type: standard, no orphan concern.
        // Local trait + foreign type: standard, callable from this crate.
        // Foreign trait + local type: allowed by the orphan rule.
        // Foreign trait + foreign type: only possible via blanket impls
        // bound to a local trait. Surfaced separately.
        let total = view.coherence_total();
        let downstream_risk = if total > 0 {
            view.local_trait_foreign_type as f64 / total as f64
        } else {
            0.0
        };

        let vector = vec![
            ("type.coherence.local_trait_local_type".into(), view.local_trait_local_type as f64),
            ("type.coherence.local_trait_foreign_type".into(), view.local_trait_foreign_type as f64),
            ("type.coherence.foreign_trait_local_type".into(), view.foreign_trait_local_type as f64),
            ("type.coherence.foreign_trait_foreign_type".into(), view.foreign_trait_foreign_type as f64),
            ("type.coherence.blanket_impl_count".into(), view.blanket_impl_count as f64),
            ("type.coherence.downstream_impl_risk_ratio".into(), downstream_risk),
            ("type.coherence.distinct_foreign_traits_implemented".into(), view.distinct_foreign_traits as f64),
            ("type.coherence.distinct_foreign_types_implemented_on".into(), view.distinct_foreign_types as f64),
        ];
        Ok(LevelOutput { vector, ..Default::default() })
    }
}

/// Aggregate stats over `impl` items in the rustdoc index, plus the
/// trait/type id sets the coherence level needs.
#[derive(Debug, Default)]
struct ImplView {
    impl_count: u64,
    inherent_impl_count: u64,
    trait_impl_count: u64,
    blanket_impl_count: u64,
    synthetic_impl_count: u64,
    negative_impl_count: u64,

    local_trait_count: u64,

    distinct_impl_traits: u64,
    distinct_impl_types: u64,

    // Coherence quadrants (non-blanket trait impls).
    local_trait_local_type: u64,
    local_trait_foreign_type: u64,
    foreign_trait_local_type: u64,
    foreign_trait_foreign_type: u64,

    distinct_foreign_traits: u64,
    distinct_foreign_types: u64,
}

impl ImplView {
    fn coherence_total(&self) -> u64 {
        self.local_trait_local_type
            + self.local_trait_foreign_type
            + self.foreign_trait_local_type
            + self.foreign_trait_foreign_type
    }

    fn collect(doc: &RustdocJson) -> Self {
        let mut view = ImplView::default();

        // Count local traits — every `paths` entry with kind=="trait" and
        // crate_id==0 is one the documented crate defines.
        view.local_trait_count = doc
            .paths
            .values()
            .filter(|s| s.crate_id == 0 && s.kind == "trait")
            .count() as u64;

        let mut impl_trait_ids: BTreeSet<String> = BTreeSet::new();
        let mut impl_type_ids: BTreeSet<ImplTypeKey> = BTreeSet::new();
        let mut foreign_trait_ids: BTreeSet<String> = BTreeSet::new();
        let mut foreign_type_ids: BTreeSet<String> = BTreeSet::new();

        for item in doc.index.values() {
            let Some(impl_obj) = extract_impl(item) else { continue };
            view.impl_count += 1;

            if impl_obj.synthetic {
                view.synthetic_impl_count += 1;
            }
            if impl_obj.negative {
                view.negative_impl_count += 1;
            }
            if impl_obj.blanket_impl_present {
                view.blanket_impl_count += 1;
            }

            match impl_obj.trait_id.as_deref() {
                None => {
                    view.inherent_impl_count += 1;
                }
                Some(tid) => {
                    view.trait_impl_count += 1;
                    impl_trait_ids.insert(tid.to_string());
                    let trait_is_local = doc.is_local_path(tid);

                    // Coherence (level 19) asks about user-written impls.
                    // Auto-trait impls (`is_synthetic`) and blanket impls
                    // aren't subject to the orphan rule in any meaningful
                    // sense — they're applied or derived automatically.
                    // Excluding them keeps the quadrant counts a clean
                    // signal about deliberate orphan-rule structure.
                    let is_user_written = !impl_obj.blanket_impl_present && !impl_obj.synthetic;
                    if is_user_written {
                        if !trait_is_local {
                            foreign_trait_ids.insert(tid.to_string());
                        }
                        let type_is_local = match &impl_obj.for_kind {
                            ForKind::ResolvedId(id) => {
                                impl_type_ids.insert(ImplTypeKey::Id(id.clone()));
                                if !doc.is_local_path(id) {
                                    foreign_type_ids.insert(id.clone());
                                }
                                doc.is_local_path(id)
                            }
                            ForKind::Primitive(name) => {
                                impl_type_ids.insert(ImplTypeKey::Primitive(name.clone()));
                                foreign_type_ids.insert(format!("primitive::{name}"));
                                false
                            }
                            ForKind::Other => {
                                impl_type_ids.insert(ImplTypeKey::Other);
                                false
                            }
                        };
                        match (trait_is_local, type_is_local) {
                            (true, true) => view.local_trait_local_type += 1,
                            (true, false) => view.local_trait_foreign_type += 1,
                            (false, true) => view.foreign_trait_local_type += 1,
                            (false, false) => view.foreign_trait_foreign_type += 1,
                        }
                    }
                }
            }

            if impl_obj.trait_id.is_none() {
                // inherent — still record the `for_` type as an "implemented
                // upon" type so distinct_impl_types reflects the full surface.
                match &impl_obj.for_kind {
                    ForKind::ResolvedId(id) => { impl_type_ids.insert(ImplTypeKey::Id(id.clone())); }
                    ForKind::Primitive(name) => { impl_type_ids.insert(ImplTypeKey::Primitive(name.clone())); }
                    ForKind::Other => { impl_type_ids.insert(ImplTypeKey::Other); }
                }
            }
        }

        view.distinct_impl_traits = impl_trait_ids.len() as u64;
        view.distinct_impl_types = impl_type_ids.len() as u64;
        view.distinct_foreign_traits = foreign_trait_ids.len() as u64;
        view.distinct_foreign_types = foreign_type_ids.len() as u64;
        view
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ImplTypeKey {
    Id(String),
    Primitive(String),
    Other,
}

/// What we need to know about an `impl` block from rustdoc JSON. We
/// extract this from the `serde_json::Value` `inner` of the index entry
/// since rustdoc tags variants by key name (`"impl"`, `"function"`, …)
/// rather than a uniform discriminant.
struct ImplFacts {
    trait_id: Option<String>,
    for_kind: ForKind,
    blanket_impl_present: bool,
    synthetic: bool,
    negative: bool,
}

enum ForKind {
    /// `for_: { resolved_path: { id: <n>, ... } }` — joins back to `paths`.
    ResolvedId(String),
    /// `for_: { primitive: "i32" }`.
    Primitive(String),
    /// Anything else (generic param, tuple, slice, fn-pointer, etc.).
    Other,
}

fn extract_impl(item: &IndexItem) -> Option<ImplFacts> {
    let impl_obj = item.inner.get("impl")?.as_object()?;

    // Field naming has drifted across rustdoc-JSON versions: older
    // shapes use `trait_`/`for_` to dodge the Rust keyword; newer shapes
    // just use `trait`/`for`. Accept both so we survive the bump.
    let trait_id = impl_obj
        .get("trait")
        .or_else(|| impl_obj.get("trait_"))
        .and_then(|t| t.as_object())
        .and_then(|t| t.get("id"))
        .map(value_id_string);

    let for_kind = impl_obj
        .get("for")
        .or_else(|| impl_obj.get("for_"))
        .map(classify_for)
        .unwrap_or(ForKind::Other);

    let blanket_impl_present = impl_obj
        .get("blanket_impl")
        .map(|v| !v.is_null())
        .unwrap_or(false);
    let synthetic = impl_obj
        .get("is_synthetic")
        .or_else(|| impl_obj.get("synthetic"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let negative = impl_obj
        .get("is_negative")
        .or_else(|| impl_obj.get("negative"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    Some(ImplFacts { trait_id, for_kind, blanket_impl_present, synthetic, negative })
}

fn classify_for(for_value: &serde_json::Value) -> ForKind {
    let Some(obj) = for_value.as_object() else { return ForKind::Other };
    if let Some(rp) = obj.get("resolved_path").and_then(|v| v.as_object()) {
        if let Some(id) = rp.get("id") {
            return ForKind::ResolvedId(value_id_string(id));
        }
    }
    if let Some(prim) = obj.get("primitive") {
        if let Some(s) = prim.as_str() {
            return ForKind::Primitive(s.to_string());
        }
    }
    ForKind::Other
}

/// rustdoc JSON formats item ids as integers in `inner` but uses
/// stringified keys in `paths`/`index`. Normalize to the string form so
/// the maps line up.
fn value_id_string(v: &serde_json::Value) -> String {
    if let Some(n) = v.as_u64() {
        return n.to_string();
    }
    if let Some(n) = v.as_i64() {
        return n.to_string();
    }
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    v.to_string()
}

fn posix(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_for_recognizes_primitive_and_resolved_path() {
        let prim: serde_json::Value =
            serde_json::from_str(r#"{"primitive":"u32"}"#).unwrap();
        assert!(matches!(classify_for(&prim), ForKind::Primitive(p) if p == "u32"));

        let resolved: serde_json::Value =
            serde_json::from_str(r#"{"resolved_path":{"id":7}}"#).unwrap();
        assert!(matches!(classify_for(&resolved), ForKind::ResolvedId(s) if s == "7"));

        let other: serde_json::Value = serde_json::from_str(r#"{"tuple":[]}"#).unwrap();
        assert!(matches!(classify_for(&other), ForKind::Other));
    }

    #[test]
    fn summarize_handles_empty_and_singletons() {
        let (mx, mn, sm) = summarize(&[]);
        assert_eq!((mx, mn, sm), (0.0, 0.0, 0.0));
        let (mx, mn, sm) = summarize(&[5]);
        assert_eq!((mx, mn, sm), (5.0, 5.0, 5.0));
        let (mx, mn, sm) = summarize(&[1, 2, 3]);
        assert_eq!(mx, 3.0);
        assert!((mn - 2.0).abs() < 1e-9);
        assert_eq!(sm, 6.0);
    }
}
