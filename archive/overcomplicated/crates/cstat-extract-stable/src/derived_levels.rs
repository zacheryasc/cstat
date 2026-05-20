//! Stage 14 — derived cross-level extractors.
//!
//! These levels are pure compositions over previously extracted
//! artifacts; they don't invoke any new tooling. Per the dev plan,
//! level 33 (inter-crate / monomorph call graph) joins level 26's
//! demangled symbol table with level 20's LLVM IR `call` instructions,
//! and level 35 (data-flow per function) reads LLVM IR's SSA form to
//! build def-use chains.
//!
//! Both gate on `Config.enable_codegen` since they consume artifacts
//! the Stage-6 codegen build produces. Without it they skip with the
//! same documented reason as the codegen levels, sharing the cache.

use std::collections::{BTreeMap, BTreeSet};

use cstat_orchestrate::{Context, CostClass, EntityRecord, Level, LevelError, LevelOutput};
use cstat_schema::{IdSet, MonoId, SymbolId};

use crate::llvm_model::{IrFunctionDetail, SharedLlvmModel};

// ───── Level 33: inter-crate / monomorph call graph ──────────────────────

pub(crate) struct GraphCallMonoLevel {
    pub shared: SharedLlvmModel,
}

impl Level for GraphCallMonoLevel {
    fn id(&self) -> &'static str { "graph.call_mono" }
    fn cost_class(&self) -> CostClass { CostClass::L }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let model = self.shared.get(ctx)?;
        let funcs = &model.functions;

        // The node set is the set of LLVM functions actually defined in
        // the module(s) — i.e. monomorphizations the codegen produced.
        // `declare`'d externs are not nodes; they show up only as the
        // target of out-edges.
        let mut name_to_idx: BTreeMap<&str, usize> = BTreeMap::new();
        for (i, f) in funcs.iter().enumerate() {
            // First define wins. rustc can emit a function under multiple
            // codegen units; the post-link view sees one node per
            // mangled name.
            name_to_idx.entry(f.name.as_str()).or_insert(i);
        }
        let node_count = name_to_idx.len();

        // Build directed edges: caller_idx → callee_idx, deduplicated.
        // Out-of-graph targets (external `@`-symbols) are tallied
        // separately so consumers can tell the in-graph fan-out from
        // the calls that crossed the crate boundary.
        let mut edges: BTreeSet<(usize, usize)> = BTreeSet::new();
        let mut external_call_count: u64 = 0;
        let mut external_targets: BTreeSet<&str> = BTreeSet::new();
        let mut total_callsites: u64 = 0;
        for f in funcs {
            let Some(&caller_idx) = name_to_idx.get(f.name.as_str()) else { continue };
            for callee in &f.callees {
                total_callsites += 1;
                match name_to_idx.get(callee.as_str()) {
                    Some(&callee_idx) => {
                        edges.insert((caller_idx, callee_idx));
                    }
                    None => {
                        external_call_count += 1;
                        external_targets.insert(callee.as_str());
                    }
                }
            }
        }
        let edge_count = edges.len() as u64;

        // Degree distributions (only over the in-graph node set).
        let mut in_deg = vec![0u64; node_count];
        let mut out_deg = vec![0u64; node_count];
        for &(c, t) in &edges {
            out_deg[c] += 1;
            in_deg[t] += 1;
        }
        let (in_max, in_mean) = max_mean(&in_deg);
        let (out_max, out_mean) = max_mean(&out_deg);

        // Recursion: count nodes with a self-edge.
        let recursive_nodes = edges.iter().filter(|(c, t)| c == t).count() as u64;
        // SCCs > 1 is harder to compute here and largely subsumed by
        // graph.sccs at the static-call-graph level. The mono graph's
        // load-bearing scalar is in/out degree shape, not SCCs again.

        let leaf_count = out_deg.iter().filter(|d| **d == 0).count() as u64;
        let root_count = in_deg.iter().filter(|d| **d == 0).count() as u64;

        let density = if node_count > 1 {
            edge_count as f64 / (node_count as f64 * (node_count as f64 - 1.0))
        } else {
            0.0
        };

        let vector = vec![
            ("graph.call_mono.node_count".into(), node_count as f64),
            ("graph.call_mono.edge_count".into(), edge_count as f64),
            ("graph.call_mono.density".into(), density),
            ("graph.call_mono.leaf_count".into(), leaf_count as f64),
            ("graph.call_mono.root_count".into(), root_count as f64),
            ("graph.call_mono.recursive_node_count".into(), recursive_nodes as f64),
            ("graph.call_mono.total_callsite_count".into(), total_callsites as f64),
            ("graph.call_mono.external_call_count".into(), external_call_count as f64),
            ("graph.call_mono.distinct_external_target_count".into(), external_targets.len() as f64),
            ("graph.call_mono.in_degree.max".into(), in_max),
            ("graph.call_mono.in_degree.mean".into(), in_mean),
            ("graph.call_mono.out_degree.max".into(), out_max),
            ("graph.call_mono.out_degree.mean".into(), out_mean),
        ];

        // Per-mono-node entities. The mono-name is the (mangled) LLVM
        // function name; demangling gives the source-level identity that
        // matches `sym.demangled`'s view, so we carry both the SymbolId
        // (the mangled form) and the MonoId (the demangled form) per
        // spec §6.
        let mut records = Vec::with_capacity(node_count);
        for (name, &idx) in &name_to_idx {
            let mut metrics = BTreeMap::new();
            metrics.insert("in_degree".into(), in_deg[idx] as f64);
            metrics.insert("out_degree".into(), out_deg[idx] as f64);
            metrics.insert(
                "callsite_count".into(),
                funcs[idx].callees.len() as f64,
            );
            metrics.insert(
                "instruction_count".into(),
                funcs[idx].instruction_count as f64,
            );
            let symbol = SymbolId::new((*name).to_string());
            let mono = demangle_to_mono_id(name);
            records.push(EntityRecord {
                ids: IdSet {
                    symbol: Some(symbol),
                    mono: Some(mono),
                    ..Default::default()
                },
                metrics,
            });
        }
        let mut entities = BTreeMap::new();
        if !records.is_empty() {
            entities.insert("graph.call_mono_node".into(), records);
        }

        Ok(LevelOutput { vector, entities })
    }
}

fn max_mean(values: &[u64]) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }
    let max = *values.iter().max().unwrap();
    let sum: u64 = values.iter().sum();
    let mean = sum as f64 / values.len() as f64;
    (max as f64, mean)
}

/// Best-effort demangle of an LLVM symbol name. Falls back to the
/// mangled string on failure so the MonoId always has *some* value —
/// consumers can dedup on whichever form they prefer.
fn demangle_to_mono_id(mangled: &str) -> MonoId {
    match rustc_demangle::try_demangle(mangled) {
        Ok(d) => MonoId::new(format!("{d:#}")),
        Err(_) => MonoId::new(mangled.to_string()),
    }
}

// ───── Level 35: per-function data-flow ──────────────────────────────────

pub(crate) struct GraphDataflowLevel {
    pub shared: SharedLlvmModel,
}

impl Level for GraphDataflowLevel {
    fn id(&self) -> &'static str { "graph.dataflow" }
    fn cost_class(&self) -> CostClass { CostClass::L }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let model = self.shared.get(ctx)?;
        let funcs = &model.functions;

        let n = funcs.len() as f64;
        let mut def_total: u64 = 0;
        let mut def_max: u64 = 0;
        let mut use_total: u64 = 0;
        let mut use_max: u64 = 0;
        let mut edge_total: u64 = 0;
        let mut edge_max: u64 = 0;
        let mut density_sum: f64 = 0.0;
        let mut density_max: f64 = 0.0;

        for f in funcs {
            let defs = f.defs.len() as u64;
            def_total += defs;
            if defs > def_max {
                def_max = defs;
            }
            use_total += f.use_count;
            if f.use_count > use_max {
                use_max = f.use_count;
            }
            edge_total += f.def_use_edge_count;
            if f.def_use_edge_count > edge_max {
                edge_max = f.def_use_edge_count;
            }
            let density = density_for(f);
            density_sum += density;
            if density > density_max {
                density_max = density;
            }
        }
        let density_mean = if n > 0.0 { density_sum / n } else { 0.0 };
        let mean_or_zero = |sum: u64| if n > 0.0 { sum as f64 / n } else { 0.0 };

        let vector = vec![
            ("graph.dataflow.function_count".into(), n),
            ("graph.dataflow.def_count.sum".into(), def_total as f64),
            ("graph.dataflow.def_count.mean".into(), mean_or_zero(def_total)),
            ("graph.dataflow.def_count.max".into(), def_max as f64),
            ("graph.dataflow.use_count.sum".into(), use_total as f64),
            ("graph.dataflow.use_count.mean".into(), mean_or_zero(use_total)),
            ("graph.dataflow.use_count.max".into(), use_max as f64),
            ("graph.dataflow.edge_count.sum".into(), edge_total as f64),
            ("graph.dataflow.edge_count.mean".into(), mean_or_zero(edge_total)),
            ("graph.dataflow.edge_count.max".into(), edge_max as f64),
            ("graph.dataflow.density.mean".into(), density_mean),
            ("graph.dataflow.density.max".into(), density_max),
        ];

        let mut records = Vec::with_capacity(funcs.len());
        for f in funcs {
            let mut metrics = BTreeMap::new();
            metrics.insert("def_count".into(), f.defs.len() as f64);
            metrics.insert("use_count".into(), f.use_count as f64);
            metrics.insert("def_use_edge_count".into(), f.def_use_edge_count as f64);
            metrics.insert("instruction_count".into(), f.instruction_count as f64);
            metrics.insert("density".into(), density_for(f));
            records.push(EntityRecord {
                ids: IdSet {
                    symbol: Some(SymbolId::new(f.name.clone())),
                    ..Default::default()
                },
                metrics,
            });
        }
        let mut entities = BTreeMap::new();
        if !records.is_empty() {
            entities.insert("graph.dataflow_func".into(), records);
        }

        Ok(LevelOutput { vector, entities })
    }
}

/// Per-function dataflow density: def-use edges per instruction. A
/// straight-line, low-reuse function lands near 1.0; tight numeric
/// kernels with heavy reuse climb above. Zero-instruction functions
/// (rare but possible — empty bodies after optimization) yield 0.
fn density_for(f: &IrFunctionDetail) -> f64 {
    if f.instruction_count == 0 {
        0.0
    } else {
        f.def_use_edge_count as f64 / f.instruction_count as f64
    }
}

// Avoid an unused-import warning when the test config doesn't reach
// some import.
#[allow(dead_code)]
fn _imports_used(_: &BTreeSet<u8>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llvm_model::IrFunctionDetail;

    fn fdetail(name: &str, callees: &[&str], inst: u64, defs: &[&str], edges: u64, uses: u64) -> IrFunctionDetail {
        IrFunctionDetail {
            name: name.into(),
            instruction_count: inst,
            basic_block_count: 1,
            callees: callees.iter().map(|s| s.to_string()).collect(),
            defs: defs.iter().map(|s| s.to_string()).collect(),
            def_use_edge_count: edges,
            use_count: uses,
        }
    }

    #[test]
    fn density_for_zero_instructions_is_zero() {
        let f = fdetail("empty", &[], 0, &[], 0, 0);
        assert_eq!(density_for(&f), 0.0);
    }

    #[test]
    fn density_for_balanced_function_matches_ratio() {
        let f = fdetail("f", &[], 4, &["a", "b"], 2, 2);
        assert_eq!(density_for(&f), 0.5);
    }

    #[test]
    fn demangle_to_mono_id_falls_back_to_raw_string_on_failure() {
        let id = demangle_to_mono_id("not_a_mangled_name");
        assert_eq!(id.as_str(), "not_a_mangled_name");
    }
}
