//! Stable-side Level impls backed by the nightly `cstat-extract-rustc`
//! sub-binary (plan Stage 13).
//!
//! Each impl reads the shared [`SharedIrDump`] cache and translates one
//! IR section of the [`cstat_ir::CstatIrDump`] into the spec §7 vector.
//! All seven levels (`ast.resolved`, `hir`, `thir`, `mir.preopt`,
//! `mir.opt`, `borrowck`, `type.mono`) share the same dump and the same
//! skip/fail reason — when nightly is unavailable, all seven report
//! `Skipped` with the standard wording; the consumer sees the contract
//! per spec §11 without inspecting the cause more than once.

use std::collections::{BTreeMap, BTreeSet};

use cstat_ir::{BorrowckDump, HirDump, MirDump, MonoItemsDump, ThirDump};
use cstat_orchestrate::{Context, CostClass, EntityRecord, Level, LevelError, LevelOutput};
use cstat_schema::IdSet;

use crate::ir_cache::SharedIrDump;

// ───── Level 10: name-resolved AST (import graph, shadowing) ─────────────

pub(crate) struct AstResolvedLevel {
    pub shared: SharedIrDump,
}

impl Level for AstResolvedLevel {
    fn id(&self) -> &'static str { "ast.resolved" }
    fn cost_class(&self) -> CostClass { CostClass::B }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let dump = self.shared.get(ctx)?;
        let Some(hir) = dump.hir.as_ref() else {
            return Err(LevelError::Failed(
                "nightly extractor returned no HIR section".into(),
            ));
        };
        Ok(build_resolved_output(hir))
    }
}

fn build_resolved_output(hir: &HirDump) -> LevelOutput {
    let total_imports = hir.imports.len() as u64;
    let mut by_kind: BTreeMap<&str, u64> = BTreeMap::new();
    let mut modules_with_imports: BTreeSet<&str> = BTreeSet::new();
    let mut distinct_targets: BTreeSet<&str> = BTreeSet::new();
    for imp in &hir.imports {
        *by_kind.entry(imp.kind.as_str()).or_default() += 1;
        modules_with_imports.insert(&imp.from_module);
        distinct_targets.insert(&imp.resolved_def_path);
    }
    let single_imports = by_kind.get("single").copied().unwrap_or(0);
    let glob_imports = by_kind.get("glob").copied().unwrap_or(0);
    let renamed_imports = by_kind.get("renamed").copied().unwrap_or(0);

    let binding_count = hir.bindings.len() as u64;
    let shadowing_bindings = hir
        .bindings
        .iter()
        .filter(|b| b.shadow_index > 0)
        .count() as u64;
    let mut distinct_binding_names: BTreeSet<&str> = BTreeSet::new();
    let mut max_shadow_index: u32 = 0;
    for b in &hir.bindings {
        distinct_binding_names.insert(b.name.as_str());
        max_shadow_index = max_shadow_index.max(b.shadow_index);
    }

    let vector = vec![
        ("ast.resolved.import_count".into(), total_imports as f64),
        ("ast.resolved.single_import_count".into(), single_imports as f64),
        ("ast.resolved.glob_import_count".into(), glob_imports as f64),
        ("ast.resolved.renamed_import_count".into(), renamed_imports as f64),
        ("ast.resolved.modules_with_imports".into(), modules_with_imports.len() as f64),
        ("ast.resolved.distinct_import_targets".into(), distinct_targets.len() as f64),
        ("ast.resolved.binding_count".into(), binding_count as f64),
        ("ast.resolved.shadowing_binding_count".into(), shadowing_bindings as f64),
        ("ast.resolved.distinct_binding_names".into(), distinct_binding_names.len() as f64),
        ("ast.resolved.max_shadow_index".into(), max_shadow_index as f64),
    ];
    LevelOutput { vector, ..Default::default() }
}

// ───── Level 11: HIR ─────────────────────────────────────────────────────

pub(crate) struct HirLevel {
    pub shared: SharedIrDump,
}

impl Level for HirLevel {
    fn id(&self) -> &'static str { "hir" }
    fn cost_class(&self) -> CostClass { CostClass::B }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let dump = self.shared.get(ctx)?;
        let Some(hir) = dump.hir.as_ref() else {
            return Err(LevelError::Failed(
                "nightly extractor returned no HIR section".into(),
            ));
        };
        Ok(build_hir_output(hir))
    }
}

fn build_hir_output(hir: &HirDump) -> LevelOutput {
    let item_count = hir.items.len() as u64;
    let fn_count = hir
        .items
        .iter()
        .filter(|i| matches!(i.kind.as_str(), "fn" | "method"))
        .count() as u64;

    let mut kind_histogram: BTreeMap<&str, u64> = BTreeMap::new();
    for item in &hir.items {
        *kind_histogram.entry(item.kind.as_str()).or_default() += 1;
    }

    let branches: Vec<u32> = hir.items.iter().map(|i| i.branch_count).collect();
    let exprs: Vec<u32> = hir.items.iter().map(|i| i.expr_count).collect();

    let (br_max, br_mean, br_sum) = summarise(&branches);
    let (ex_max, ex_mean, ex_sum) = summarise(&exprs);

    // Roll the per-item expr_kinds histograms together. The set of
    // strings is whatever the nightly translator emitted on this host —
    // new variants land as new keys, no schema bump required.
    let mut global_expr_kinds: BTreeMap<&str, u64> = BTreeMap::new();
    for item in &hir.items {
        for (kind, count) in &item.expr_kinds {
            *global_expr_kinds.entry(kind.as_str()).or_default() += *count as u64;
        }
    }

    let mut vector = vec![
        ("hir.item_count".into(), item_count as f64),
        ("hir.fn_count".into(), fn_count as f64),
        ("hir.global.branch_count.sum".into(), br_sum),
        ("hir.global.branch_count.max".into(), br_max),
        ("hir.global.branch_count.mean".into(), br_mean),
        ("hir.global.expr_count.sum".into(), ex_sum),
        ("hir.global.expr_count.max".into(), ex_max),
        ("hir.global.expr_count.mean".into(), ex_mean),
        ("hir.distinct_expr_kinds".into(), global_expr_kinds.len() as f64),
    ];
    for (kind, count) in &global_expr_kinds {
        vector.push((format!("hir.expr_kind.{kind}"), *count as f64));
    }
    for (kind, count) in &kind_histogram {
        vector.push((format!("hir.item_kind.{kind}"), *count as f64));
    }

    // Per-fn HIR records carry the rolled-up branch/expression counts
    // but no IdSet — joining HIR def-paths to `AstFuncId` (which is
    // structurally hashed by syn) is a Stage-14 derived-level concern,
    // not something this Level should fabricate.
    let mut entities: Vec<EntityRecord> = Vec::new();
    for item in &hir.items {
        if !matches!(item.kind.as_str(), "fn" | "method") {
            continue;
        }
        let mut metrics = BTreeMap::new();
        metrics.insert("branch_count".into(), item.branch_count as f64);
        metrics.insert("expr_count".into(), item.expr_count as f64);
        entities.push(EntityRecord {
            ids: IdSet::default(),
            metrics,
        });
    }

    let mut entity_map: BTreeMap<String, Vec<EntityRecord>> = BTreeMap::new();
    if !entities.is_empty() {
        entity_map.insert("hir.func".into(), entities);
    }

    LevelOutput { vector, entities: entity_map }
}

// ───── Level 12: THIR ────────────────────────────────────────────────────

pub(crate) struct ThirLevel {
    pub shared: SharedIrDump,
}

impl Level for ThirLevel {
    fn id(&self) -> &'static str { "thir" }
    fn cost_class(&self) -> CostClass { CostClass::B }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let dump = self.shared.get(ctx)?;
        let Some(thir) = dump.thir.as_ref() else {
            return Err(LevelError::Failed(
                "nightly extractor returned no THIR section".into(),
            ));
        };
        Ok(build_thir_output(thir))
    }
}

fn build_thir_output(thir: &ThirDump) -> LevelOutput {
    let body_count = thir.bodies.len() as u64;
    let distinct_types: Vec<u32> = thir.bodies.iter().map(|b| b.distinct_type_count).collect();
    let expr_counts: Vec<u32> = thir.bodies.iter().map(|b| b.expr_count).collect();
    let generic_arg_totals: Vec<u32> =
        thir.bodies.iter().map(|b| b.generic_arg_total).collect();
    let generic_exprs: Vec<u32> = thir.bodies.iter().map(|b| b.generic_expr_count).collect();
    let trait_objects: Vec<u32> = thir.bodies.iter().map(|b| b.trait_object_count).collect();

    let (dt_max, dt_mean, dt_sum) = summarise(&distinct_types);
    let (ex_max, ex_mean, ex_sum) = summarise(&expr_counts);
    let (ga_max, ga_mean, ga_sum) = summarise(&generic_arg_totals);
    let (_, _, ge_sum) = summarise(&generic_exprs);
    let (_, _, to_sum) = summarise(&trait_objects);

    let total_expr = ex_sum.max(1.0);
    let polymorphism_density = ge_sum / total_expr;
    let trait_object_density = to_sum / total_expr;

    let vector = vec![
        ("thir.body_count".into(), body_count as f64),
        ("thir.global.distinct_types.sum".into(), dt_sum),
        ("thir.global.distinct_types.max".into(), dt_max),
        ("thir.global.distinct_types.mean".into(), dt_mean),
        ("thir.global.expr_count.sum".into(), ex_sum),
        ("thir.global.expr_count.max".into(), ex_max),
        ("thir.global.expr_count.mean".into(), ex_mean),
        ("thir.global.generic_arg_total.sum".into(), ga_sum),
        ("thir.global.generic_arg_total.max".into(), ga_max),
        ("thir.global.generic_arg_total.mean".into(), ga_mean),
        ("thir.global.generic_expr_count.sum".into(), ge_sum),
        ("thir.global.trait_object_count.sum".into(), to_sum),
        ("thir.global.polymorphism_density".into(), polymorphism_density),
        ("thir.global.trait_object_density".into(), trait_object_density),
    ];

    let mut entities: Vec<EntityRecord> = Vec::new();
    for body in &thir.bodies {
        let mut metrics = BTreeMap::new();
        metrics.insert("expr_count".into(), body.expr_count as f64);
        metrics.insert("distinct_types".into(), body.distinct_type_count as f64);
        metrics.insert("generic_arg_total".into(), body.generic_arg_total as f64);
        metrics.insert("generic_expr_count".into(), body.generic_expr_count as f64);
        metrics.insert("trait_object_count".into(), body.trait_object_count as f64);
        entities.push(EntityRecord {
            ids: IdSet::default(),
            metrics,
        });
    }
    let mut entity_map = BTreeMap::new();
    if !entities.is_empty() {
        entity_map.insert("thir.body".into(), entities);
    }
    LevelOutput { vector, entities: entity_map }
}

// ───── Levels 13 + 14: MIR (pre-opt and optimized) ───────────────────────

pub(crate) struct MirPreoptLevel {
    pub shared: SharedIrDump,
}

impl Level for MirPreoptLevel {
    fn id(&self) -> &'static str { "mir.preopt" }
    fn cost_class(&self) -> CostClass { CostClass::B }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let dump = self.shared.get(ctx)?;
        let Some(mir) = dump.mir_preopt.as_ref() else {
            return Err(LevelError::Failed(
                "nightly extractor returned no pre-opt MIR section".into(),
            ));
        };
        Ok(build_mir_output("mir.preopt", mir))
    }
}

pub(crate) struct MirOptLevel {
    pub shared: SharedIrDump,
}

impl Level for MirOptLevel {
    fn id(&self) -> &'static str { "mir.opt" }
    fn cost_class(&self) -> CostClass { CostClass::B }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let dump = self.shared.get(ctx)?;
        let Some(mir) = dump.mir_opt.as_ref() else {
            return Err(LevelError::Failed(
                "nightly extractor returned no opt MIR section".into(),
            ));
        };
        Ok(build_mir_output("mir.opt", mir))
    }
}

fn build_mir_output(level_id: &str, mir: &MirDump) -> LevelOutput {
    let body_count = mir.bodies.len() as u64;

    let block_counts: Vec<u32> = mir
        .bodies
        .iter()
        .map(|b| b.basic_blocks.len() as u32)
        .collect();
    let local_counts: Vec<u32> = mir.bodies.iter().map(|b| b.local_count).collect();
    let arg_counts: Vec<u32> = mir.bodies.iter().map(|b| b.arg_count).collect();
    let stmt_counts: Vec<u32> = mir
        .bodies
        .iter()
        .map(|b| {
            b.basic_blocks
                .iter()
                .map(|bb| bb.statement_kinds.len() as u32)
                .sum()
        })
        .collect();

    let (bb_max, bb_mean, bb_sum) = summarise(&block_counts);
    let (lc_max, lc_mean, lc_sum) = summarise(&local_counts);
    let (_, _, ac_sum) = summarise(&arg_counts);
    let (_, st_mean, st_sum) = summarise(&stmt_counts);

    // Statement-kind and terminator histograms across the whole crate.
    let mut stmt_kinds: BTreeMap<&str, u64> = BTreeMap::new();
    let mut term_kinds: BTreeMap<&str, u64> = BTreeMap::new();
    for body in &mir.bodies {
        for bb in &body.basic_blocks {
            for kind in &bb.statement_kinds {
                *stmt_kinds.entry(kind.as_str()).or_default() += 1;
            }
            *term_kinds.entry(bb.terminator.as_str()).or_default() += 1;
        }
    }

    let mut vector = vec![
        (format!("{level_id}.body_count"), body_count as f64),
        (format!("{level_id}.basic_block_count.sum"), bb_sum),
        (format!("{level_id}.basic_block_count.max"), bb_max),
        (format!("{level_id}.basic_block_count.mean"), bb_mean),
        (format!("{level_id}.local_count.sum"), lc_sum),
        (format!("{level_id}.local_count.max"), lc_max),
        (format!("{level_id}.local_count.mean"), lc_mean),
        (format!("{level_id}.arg_count.sum"), ac_sum),
        (format!("{level_id}.statement_count.sum"), st_sum),
        (format!("{level_id}.statement_count.mean"), st_mean),
        (
            format!("{level_id}.distinct_terminator_kinds"),
            term_kinds.len() as f64,
        ),
        (
            format!("{level_id}.distinct_statement_kinds"),
            stmt_kinds.len() as f64,
        ),
    ];
    for (kind, count) in &term_kinds {
        vector.push((format!("{level_id}.terminator_kind.{kind}"), *count as f64));
    }
    for (kind, count) in &stmt_kinds {
        vector.push((format!("{level_id}.statement_kind.{kind}"), *count as f64));
    }

    let entity_kind = if level_id == "mir.preopt" {
        "mir.preopt_body"
    } else {
        "mir.opt_body"
    };
    let mut entities: Vec<EntityRecord> = Vec::new();
    for body in &mir.bodies {
        let mut metrics = BTreeMap::new();
        metrics.insert("basic_blocks".into(), body.basic_blocks.len() as f64);
        metrics.insert("arg_count".into(), body.arg_count as f64);
        metrics.insert("local_count".into(), body.local_count as f64);
        let stmt_total: u32 = body
            .basic_blocks
            .iter()
            .map(|bb| bb.statement_kinds.len() as u32)
            .sum();
        metrics.insert("statement_count".into(), stmt_total as f64);
        entities.push(EntityRecord {
            ids: IdSet::default(),
            metrics,
        });
    }
    let mut entity_map = BTreeMap::new();
    if !entities.is_empty() {
        entity_map.insert(entity_kind.into(), entities);
    }

    LevelOutput { vector, entities: entity_map }
}

// ───── Level 15: borrowck ────────────────────────────────────────────────

pub(crate) struct BorrowckLevel {
    pub shared: SharedIrDump,
}

impl Level for BorrowckLevel {
    fn id(&self) -> &'static str { "borrowck" }
    fn cost_class(&self) -> CostClass { CostClass::B }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let dump = self.shared.get(ctx)?;
        let Some(bc) = dump.borrowck.as_ref() else {
            return Err(LevelError::Failed(
                "nightly extractor returned no borrowck section".into(),
            ));
        };
        Ok(build_borrowck_output(bc))
    }
}

fn build_borrowck_output(bc: &BorrowckDump) -> LevelOutput {
    let body_count = bc.bodies.len() as u64;
    let regions: Vec<u32> = bc.bodies.iter().map(|b| b.region_count).collect();
    let outlives: Vec<u32> = bc
        .bodies
        .iter()
        .map(|b| b.outlives_constraint_count)
        .collect();
    let members: Vec<u32> = bc
        .bodies
        .iter()
        .map(|b| b.member_constraint_count)
        .collect();
    let nll: Vec<u32> = bc.bodies.iter().map(|b| b.nll_conflict_count).collect();

    let (rg_max, rg_mean, rg_sum) = summarise(&regions);
    let (_, _, ol_sum) = summarise(&outlives);
    let (_, _, mb_sum) = summarise(&members);
    let (_, _, nl_sum) = summarise(&nll);

    let vector = vec![
        ("borrowck.body_count".into(), body_count as f64),
        ("borrowck.region_count.sum".into(), rg_sum),
        ("borrowck.region_count.max".into(), rg_max),
        ("borrowck.region_count.mean".into(), rg_mean),
        ("borrowck.outlives_constraint_count.sum".into(), ol_sum),
        ("borrowck.member_constraint_count.sum".into(), mb_sum),
        ("borrowck.nll_conflict_count.sum".into(), nl_sum),
    ];

    let mut entities: Vec<EntityRecord> = Vec::new();
    for body in &bc.bodies {
        let mut metrics = BTreeMap::new();
        metrics.insert("region_count".into(), body.region_count as f64);
        metrics.insert("outlives_constraints".into(), body.outlives_constraint_count as f64);
        metrics.insert("member_constraints".into(), body.member_constraint_count as f64);
        metrics.insert("nll_conflicts".into(), body.nll_conflict_count as f64);
        entities.push(EntityRecord {
            ids: IdSet::default(),
            metrics,
        });
    }
    let mut entity_map = BTreeMap::new();
    if !entities.is_empty() {
        entity_map.insert("borrowck.body".into(), entities);
    }
    LevelOutput { vector, entities: entity_map }
}

// ───── Level 18: type.mono (generic instantiation map) ───────────────────

pub(crate) struct TypeMonoLevel {
    pub shared: SharedIrDump,
}

impl Level for TypeMonoLevel {
    fn id(&self) -> &'static str { "type.mono" }
    fn cost_class(&self) -> CostClass { CostClass::B }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let dump = self.shared.get(ctx)?;
        let Some(mono) = dump.mono_items.as_ref() else {
            return Err(LevelError::Failed(
                "nightly extractor returned no mono-items section".into(),
            ));
        };
        Ok(build_mono_output(mono))
    }
}

fn build_mono_output(mono: &MonoItemsDump) -> LevelOutput {
    let item_count = mono.items.len() as u64;
    let mut by_kind: BTreeMap<&str, u64> = BTreeMap::new();
    for item in &mono.items {
        *by_kind.entry(item.kind.as_str()).or_default() += 1;
    }

    let counts: Vec<u32> = mono
        .generic_instantiation_groups
        .iter()
        .map(|g| g.instantiation_count)
        .collect();
    let (ic_max, ic_mean, ic_sum) = summarise(&counts);
    let multi_instance_groups = mono
        .generic_instantiation_groups
        .iter()
        .filter(|g| g.instantiation_count > 1)
        .count() as u64;

    let mut vector = vec![
        ("type.mono.item_count".into(), item_count as f64),
        ("type.mono.distinct_kinds".into(), by_kind.len() as f64),
        ("type.mono.generic_group_count".into(), mono.generic_instantiation_groups.len() as f64),
        ("type.mono.instantiation_count.sum".into(), ic_sum),
        ("type.mono.instantiation_count.max".into(), ic_max),
        ("type.mono.instantiation_count.mean".into(), ic_mean),
        ("type.mono.multi_instance_group_count".into(), multi_instance_groups as f64),
    ];
    for (kind, count) in &by_kind {
        vector.push((format!("type.mono.kind.{kind}"), *count as f64));
    }
    LevelOutput { vector, ..Default::default() }
}

// ───── helpers ───────────────────────────────────────────────────────────

fn summarise(values: &[u32]) -> (f64, f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let sum: u64 = values.iter().map(|&v| v as u64).sum();
    let max = values.iter().copied().max().unwrap_or(0) as f64;
    let mean = sum as f64 / values.len() as f64;
    (max, mean, sum as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cstat_ir::{
        BorrowckBody, GenericInstantiationGroup, HirBinding, HirImport, HirItem, MirBasicBlock,
        MirBody, MonoItem, ThirBody,
    };

    #[test]
    fn ast_resolved_counts_import_kinds_and_shadowing() {
        let hir = HirDump {
            items: vec![],
            imports: vec![
                HirImport {
                    from_module: "crate".into(),
                    resolved_def_path: "std::fmt::Display".into(),
                    kind: "single".into(),
                },
                HirImport {
                    from_module: "crate::sub".into(),
                    resolved_def_path: "std::io::Write".into(),
                    kind: "single".into(),
                },
                HirImport {
                    from_module: "crate::sub".into(),
                    resolved_def_path: "std::collections::*".into(),
                    kind: "glob".into(),
                },
                HirImport {
                    from_module: "crate".into(),
                    resolved_def_path: "core::option::Option".into(),
                    kind: "renamed".into(),
                },
            ],
            bindings: vec![
                HirBinding { name: "x".into(), scope_def_path: "crate::f".into(), shadow_index: 0 },
                HirBinding { name: "x".into(), scope_def_path: "crate::f".into(), shadow_index: 1 },
                HirBinding { name: "y".into(), scope_def_path: "crate::f".into(), shadow_index: 0 },
            ],
        };
        let out = build_resolved_output(&hir);

        let v: BTreeMap<_, _> = out.vector.into_iter().collect();
        assert_eq!(v.get("ast.resolved.import_count").copied(), Some(4.0));
        assert_eq!(v.get("ast.resolved.single_import_count").copied(), Some(2.0));
        assert_eq!(v.get("ast.resolved.glob_import_count").copied(), Some(1.0));
        assert_eq!(v.get("ast.resolved.renamed_import_count").copied(), Some(1.0));
        assert_eq!(v.get("ast.resolved.modules_with_imports").copied(), Some(2.0));
        assert_eq!(v.get("ast.resolved.distinct_import_targets").copied(), Some(4.0));
        assert_eq!(v.get("ast.resolved.binding_count").copied(), Some(3.0));
        assert_eq!(v.get("ast.resolved.shadowing_binding_count").copied(), Some(1.0));
        assert_eq!(v.get("ast.resolved.distinct_binding_names").copied(), Some(2.0));
        assert_eq!(v.get("ast.resolved.max_shadow_index").copied(), Some(1.0));
    }

    #[test]
    fn hir_rolls_up_expr_kinds_across_items() {
        let mut k1 = BTreeMap::new();
        k1.insert("if".to_string(), 2u32);
        k1.insert("method_call".to_string(), 3u32);
        let mut k2 = BTreeMap::new();
        k2.insert("if".to_string(), 1u32);
        k2.insert("loop".to_string(), 1u32);
        let hir = HirDump {
            items: vec![
                HirItem {
                    def_path: "a".into(),
                    kind: "fn".into(),
                    branch_count: 2,
                    expr_count: 10,
                    expr_kinds: k1,
                },
                HirItem {
                    def_path: "b".into(),
                    kind: "fn".into(),
                    branch_count: 1,
                    expr_count: 5,
                    expr_kinds: k2,
                },
            ],
            imports: vec![],
            bindings: vec![],
        };
        let out = build_hir_output(&hir);
        let v: BTreeMap<_, _> = out.vector.into_iter().collect();
        assert_eq!(v.get("hir.item_count").copied(), Some(2.0));
        assert_eq!(v.get("hir.fn_count").copied(), Some(2.0));
        assert_eq!(v.get("hir.global.branch_count.sum").copied(), Some(3.0));
        assert_eq!(v.get("hir.global.expr_count.sum").copied(), Some(15.0));
        assert_eq!(v.get("hir.expr_kind.if").copied(), Some(3.0));
        assert_eq!(v.get("hir.expr_kind.loop").copied(), Some(1.0));
        assert_eq!(v.get("hir.expr_kind.method_call").copied(), Some(3.0));
        assert_eq!(v.get("hir.distinct_expr_kinds").copied(), Some(3.0));
    }

    #[test]
    fn thir_density_is_zero_when_no_polymorphism() {
        let thir = ThirDump {
            bodies: vec![ThirBody {
                def_path: "a".into(),
                expr_count: 10,
                distinct_type_count: 3,
                generic_arg_total: 0,
                trait_object_count: 0,
                generic_expr_count: 0,
            }],
        };
        let out = build_thir_output(&thir);
        let v: BTreeMap<_, _> = out.vector.into_iter().collect();
        assert_eq!(v.get("thir.global.polymorphism_density").copied(), Some(0.0));
        assert_eq!(v.get("thir.global.trait_object_density").copied(), Some(0.0));
        assert_eq!(v.get("thir.global.distinct_types.sum").copied(), Some(3.0));
    }

    #[test]
    fn mir_histograms_terminators_and_statements() {
        let body = MirBody {
            def_path: "f".into(),
            arg_count: 1,
            local_count: 4,
            basic_blocks: vec![
                MirBasicBlock {
                    id: 0,
                    statement_kinds: vec!["StorageLive".into(), "Assign".into()],
                    terminator: "Goto".into(),
                    predecessors: vec![],
                    successors: vec![1],
                },
                MirBasicBlock {
                    id: 1,
                    statement_kinds: vec!["Assign".into()],
                    terminator: "Return".into(),
                    predecessors: vec![0],
                    successors: vec![],
                },
            ],
        };
        let mir = MirDump { bodies: vec![body] };
        let out = build_mir_output("mir.opt", &mir);
        let v: BTreeMap<_, _> = out.vector.into_iter().collect();
        assert_eq!(v.get("mir.opt.body_count").copied(), Some(1.0));
        assert_eq!(v.get("mir.opt.basic_block_count.sum").copied(), Some(2.0));
        assert_eq!(v.get("mir.opt.statement_count.sum").copied(), Some(3.0));
        assert_eq!(v.get("mir.opt.terminator_kind.Goto").copied(), Some(1.0));
        assert_eq!(v.get("mir.opt.terminator_kind.Return").copied(), Some(1.0));
        assert_eq!(v.get("mir.opt.statement_kind.Assign").copied(), Some(2.0));
        assert_eq!(v.get("mir.opt.statement_kind.StorageLive").copied(), Some(1.0));
        assert_eq!(v.get("mir.opt.distinct_terminator_kinds").copied(), Some(2.0));
    }

    #[test]
    fn borrowck_sums_region_and_constraint_counts() {
        let bc = BorrowckDump {
            bodies: vec![
                BorrowckBody {
                    def_path: "a".into(),
                    region_count: 2,
                    outlives_constraint_count: 3,
                    member_constraint_count: 1,
                    nll_conflict_count: 0,
                },
                BorrowckBody {
                    def_path: "b".into(),
                    region_count: 4,
                    outlives_constraint_count: 1,
                    member_constraint_count: 0,
                    nll_conflict_count: 2,
                },
            ],
        };
        let out = build_borrowck_output(&bc);
        let v: BTreeMap<_, _> = out.vector.into_iter().collect();
        assert_eq!(v.get("borrowck.body_count").copied(), Some(2.0));
        assert_eq!(v.get("borrowck.region_count.sum").copied(), Some(6.0));
        assert_eq!(v.get("borrowck.region_count.max").copied(), Some(4.0));
        assert_eq!(v.get("borrowck.outlives_constraint_count.sum").copied(), Some(4.0));
        assert_eq!(v.get("borrowck.member_constraint_count.sum").copied(), Some(1.0));
        assert_eq!(v.get("borrowck.nll_conflict_count.sum").copied(), Some(2.0));
    }

    #[test]
    fn mono_groups_with_multi_instance_count_correctly() {
        let mono = MonoItemsDump {
            items: vec![
                MonoItem {
                    def_path: "a".into(),
                    kind: "fn".into(),
                    instantiation: "a".into(),
                },
                MonoItem {
                    def_path: "g".into(),
                    kind: "fn".into(),
                    instantiation: "g::<i32>".into(),
                },
                MonoItem {
                    def_path: "g".into(),
                    kind: "fn".into(),
                    instantiation: "g::<u8>".into(),
                },
                MonoItem {
                    def_path: "s".into(),
                    kind: "static".into(),
                    instantiation: "s".into(),
                },
            ],
            generic_instantiation_groups: vec![
                GenericInstantiationGroup {
                    source_def_path: "a".into(),
                    instantiation_count: 1,
                },
                GenericInstantiationGroup {
                    source_def_path: "g".into(),
                    instantiation_count: 2,
                },
                GenericInstantiationGroup {
                    source_def_path: "s".into(),
                    instantiation_count: 1,
                },
            ],
        };
        let out = build_mono_output(&mono);
        let v: BTreeMap<_, _> = out.vector.into_iter().collect();
        assert_eq!(v.get("type.mono.item_count").copied(), Some(4.0));
        assert_eq!(v.get("type.mono.distinct_kinds").copied(), Some(2.0));
        assert_eq!(v.get("type.mono.generic_group_count").copied(), Some(3.0));
        assert_eq!(v.get("type.mono.instantiation_count.sum").copied(), Some(4.0));
        assert_eq!(v.get("type.mono.instantiation_count.max").copied(), Some(2.0));
        assert_eq!(v.get("type.mono.multi_instance_group_count").copied(), Some(1.0));
        assert_eq!(v.get("type.mono.kind.fn").copied(), Some(3.0));
        assert_eq!(v.get("type.mono.kind.static").copied(), Some(1.0));
    }
}
