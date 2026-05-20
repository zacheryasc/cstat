//! Mono-item translator (spec §2.4 #18).
//!
//! Walks `collect_and_partition_mono_items` and emits one `MonoItem`
//! per (def, substitution) pair plus a per-source-def rollup.
//! `kind` is a string discriminant whose exact name comes from rustc —
//! the stable side only counts, so a new rustc variant lands as a new
//! string in the table, not a parse error.

use std::collections::BTreeMap;

use cstat_ir::{GenericInstantiationGroup, MonoItem, MonoItemsDump};
use rustc_middle::mono::MonoItem as RustcMonoItem;
use rustc_middle::ty::TyCtxt;

pub fn run(tcx: TyCtxt<'_>) -> MonoItemsDump {
    let mut items: Vec<MonoItem> = Vec::new();
    let mut groups: BTreeMap<String, u32> = BTreeMap::new();

    let result = tcx.collect_and_partition_mono_items(());
    for cgu in result.codegen_units.iter() {
        for (item, _data) in cgu.items() {
            let (def_path, kind, instantiation) = describe(tcx, item);
            *groups.entry(def_path.clone()).or_insert(0) += 1;
            items.push(MonoItem {
                def_path,
                kind: kind.to_string(),
                instantiation,
            });
        }
    }

    let mut g: Vec<GenericInstantiationGroup> = groups
        .into_iter()
        .map(|(source_def_path, instantiation_count)| GenericInstantiationGroup {
            source_def_path,
            instantiation_count,
        })
        .collect();
    g.sort_by(|a, b| a.source_def_path.cmp(&b.source_def_path));

    MonoItemsDump {
        items,
        generic_instantiation_groups: g,
    }
}

fn describe<'tcx>(tcx: TyCtxt<'tcx>, item: &RustcMonoItem<'tcx>) -> (String, &'static str, String) {
    match item {
        RustcMonoItem::Fn(instance) => {
            let def_id = instance.def_id();
            let def_path = tcx.def_path_str(def_id);
            let kind = if tcx.def_kind(def_id) == rustc_hir::def::DefKind::AssocFn {
                "method"
            } else {
                "fn"
            };
            let instantiation = instance.to_string();
            (def_path, kind, instantiation)
        }
        RustcMonoItem::Static(def_id) => {
            let def_path = tcx.def_path_str(*def_id);
            (def_path.clone(), "static", def_path)
        }
        RustcMonoItem::GlobalAsm(item_id) => {
            let def_id = item_id.owner_id.to_def_id();
            let def_path = tcx.def_path_str(def_id);
            (def_path.clone(), "global_asm", def_path)
        }
    }
}
