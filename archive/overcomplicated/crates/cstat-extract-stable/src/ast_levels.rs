//! Spec §2.2 levels 7 (CST), 8 (item tree), 9 (macro-expanded AST).
//!
//! Levels 7 and 8 share a single lazy `syn` parse + walk via [`SharedAst`].
//! Per-file parse failures land in `entities.ast.file_errors` and the file
//! is excluded from downstream AST metrics — spec §11 row 4. The
//! orchestrator counts these into `meta.files_unparseable.count`.
//!
//! Level 9 shells out to `cargo expand`; if the subcommand or nightly
//! prerequisite is missing it reports `Skipped` with a reason rather than
//! failing the run.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::time::Duration;

use cstat_orchestrate::{Context, CostClass, EntityRecord, Level, LevelError, LevelOutput};
use cstat_schema::{FileId, IdSet, ModuleId};
use proc_macro2::TokenStream;
use syn::visit::{self, Visit};

use crate::ast_cache::SharedAst;

// ───── Level 7: concrete syntax tree ────────────────────────────────────

pub(crate) struct AstCstLevel {
    pub shared: SharedAst,
}

impl Level for AstCstLevel {
    fn id(&self) -> &'static str { "ast.cst" }
    fn cost_class(&self) -> CostClass { CostClass::L }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let ast = self.shared.get(&ctx.project_root);

        let mut attribute_count = 0u64;
        let mut macro_invocations = 0u64;
        let mut all_depths: Vec<u32> = Vec::new();
        for f in &ast.files {
            if f.parse_error.is_some() {
                continue;
            }
            attribute_count += f.attribute_count;
            macro_invocations += f.macro_invocations;
            all_depths.extend_from_slice(&f.item_depths);
        }

        let parsed_ok = ast.parsed_ok_count() as f64;
        let attempted = ast.attempted();
        let unparseable = (attempted - ast.parsed_ok_count()) as f64;
        let parse_success_rate = if attempted > 0 {
            parsed_ok / attempted as f64
        } else {
            1.0
        };

        let depth_mean = if all_depths.is_empty() {
            0.0
        } else {
            all_depths.iter().sum::<u32>() as f64 / all_depths.len() as f64
        };
        let depth_max = all_depths.iter().copied().max().unwrap_or(0) as f64;

        let vector = vec![
            ("ast.cst.parsed_file_count".into(), parsed_ok),
            ("ast.cst.unparseable_file_count".into(), unparseable),
            ("ast.cst.parse_success_rate".into(), parse_success_rate),
            ("ast.cst.attribute_count".into(), attribute_count as f64),
            ("ast.cst.macro_invocation_count".into(), macro_invocations as f64),
            ("ast.cst.item_count".into(), all_depths.len() as f64),
            ("ast.cst.item_depth.max".into(), depth_max),
            ("ast.cst.item_depth.mean".into(), depth_mean),
        ];

        let mut file_errors = Vec::new();
        for (path, err) in ast.iter_errors() {
            let mut metrics = BTreeMap::new();
            metrics.insert("line".into(), err.line as f64);
            metrics.insert("column".into(), err.column as f64);
            file_errors.push(EntityRecord {
                ids: IdSet {
                    file: Some(FileId::new(posix(path))),
                    ..Default::default()
                },
                metrics,
            });
        }
        let mut entities: BTreeMap<String, Vec<EntityRecord>> = BTreeMap::new();
        if !file_errors.is_empty() {
            entities.insert("ast.file_errors".into(), file_errors);
        }

        Ok(LevelOutput { vector, entities })
    }
}

// ───── Level 8: item tree / module skeleton ─────────────────────────────

pub(crate) struct AstItemsLevel {
    pub shared: SharedAst,
}

impl Level for AstItemsLevel {
    fn id(&self) -> &'static str { "ast.items" }
    fn cost_class(&self) -> CostClass { CostClass::L }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let ast = self.shared.get(&ctx.project_root);

        let mut totals = crate::ast_cache::ItemTotals::default();
        let mut modules = Vec::new();
        for f in &ast.files {
            if f.parse_error.is_some() {
                continue;
            }
            totals.merge(&f.totals);
            modules.extend(f.modules.iter().cloned());
        }
        modules.sort_by(|a, b| {
            (posix(&a.file_path), &a.module_path).cmp(&(posix(&b.file_path), &b.module_path))
        });

        let total_items = totals.item_count();
        let pub_ratio = ratio(totals.pub_items, total_items);
        let pub_crate_ratio = ratio(totals.pub_crate_items, total_items);
        let private_ratio = ratio(totals.private_items, total_items);
        let mod_depth_max = modules.iter().map(|m| m.depth).max().unwrap_or(0);

        let vector = vec![
            ("ast.global.func_count".into(), totals.functions as f64),
            ("ast.global.struct_count".into(), totals.structs as f64),
            ("ast.global.enum_count".into(), totals.enums as f64),
            ("ast.global.trait_count".into(), totals.traits as f64),
            ("ast.global.impl_count".into(), totals.impls as f64),
            ("ast.global.use_count".into(), totals.uses as f64),
            ("ast.global.const_count".into(), totals.consts as f64),
            ("ast.global.static_count".into(), totals.statics as f64),
            ("ast.global.type_alias_count".into(), totals.type_aliases as f64),
            ("ast.items.module_count".into(), modules.len() as f64),
            ("ast.items.module_depth.max".into(), mod_depth_max as f64),
            ("ast.items.pub_ratio".into(), pub_ratio),
            ("ast.items.pub_crate_ratio".into(), pub_crate_ratio),
            ("ast.items.private_ratio".into(), private_ratio),
        ];

        let mut module_records = Vec::with_capacity(modules.len());
        for m in modules {
            let mut metrics = BTreeMap::new();
            metrics.insert("depth".into(), m.depth as f64);
            metrics.insert("func_count".into(), m.totals.functions as f64);
            metrics.insert("struct_count".into(), m.totals.structs as f64);
            metrics.insert("enum_count".into(), m.totals.enums as f64);
            metrics.insert("trait_count".into(), m.totals.traits as f64);
            metrics.insert("impl_count".into(), m.totals.impls as f64);
            metrics.insert("use_count".into(), m.totals.uses as f64);
            metrics.insert("item_count".into(), m.totals.item_count() as f64);
            module_records.push(EntityRecord {
                ids: IdSet {
                    file: Some(FileId::new(posix(&m.file_path))),
                    module: Some(ModuleId::new(m.module_path)),
                    ..Default::default()
                },
                metrics,
            });
        }
        let mut entities: BTreeMap<String, Vec<EntityRecord>> = BTreeMap::new();
        if !module_records.is_empty() {
            entities.insert("ast.module".into(), module_records);
        }

        Ok(LevelOutput { vector, entities })
    }
}

// ───── Level 9: macro-expanded AST (cargo expand) ───────────────────────

pub(crate) struct AstExpandedLevel {
    pub shared: SharedAst,
}

impl Level for AstExpandedLevel {
    fn id(&self) -> &'static str { "ast.expanded" }
    fn cost_class(&self) -> CostClass { CostClass::B }
    fn deterministic(&self) -> bool { false }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        // Touch the shared cache so a future stage that wants to compare
        // pre- vs post-expansion has an obvious place to plug in.
        let _ast = self.shared.get(&ctx.project_root);

        if !cargo_expand_available() {
            return Err(LevelError::Skipped(
                "`cargo expand` subcommand not installed (cargo-expand)".into(),
            ));
        }

        let output = Command::new("cargo")
            .arg("expand")
            .arg("--lib")
            .current_dir(&ctx.project_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();

        let result = match output {
            Ok(o) => o,
            Err(e) => return Err(LevelError::Failed(format!("invoking cargo expand: {e}"))),
        };

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            // cargo-expand requires nightly internally; surface that as a
            // skip rather than a failure so the run still reports cleanly
            // when the host lacks nightly.
            if stderr.contains("nightly") || stderr.contains("toolchain") {
                return Err(LevelError::Skipped(format!(
                    "cargo expand could not run: {}",
                    stderr.lines().next().unwrap_or("unknown error").trim()
                )));
            }
            return Err(LevelError::Failed(format!(
                "cargo expand exited with {:?}: {}",
                result.status.code(),
                stderr.lines().next().unwrap_or("").trim()
            )));
        }

        let expanded = match std::str::from_utf8(&result.stdout) {
            Ok(s) => s.to_string(),
            Err(_) => return Err(LevelError::Failed("cargo expand stdout was not UTF-8".into())),
        };

        let post_tokens = TokenStream::from_str(&expanded)
            .map(count_tokens)
            .unwrap_or(0);
        let (post_functions, post_macros) = match syn::parse_file(&expanded) {
            Ok(file) => {
                let mut v = ExpandedVisitor::default();
                v.visit_file(&file);
                (v.functions, v.macros)
            }
            Err(_) => (0, 0),
        };

        let vector = vec![
            ("ast.expanded.token_count".into(), post_tokens as f64),
            ("ast.expanded.func_count".into(), post_functions as f64),
            ("ast.expanded.macro_invocation_count".into(), post_macros as f64),
        ];

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

#[derive(Default)]
struct ExpandedVisitor {
    functions: u64,
    macros: u64,
}

impl<'ast> Visit<'ast> for ExpandedVisitor {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.functions += 1;
        visit::visit_item_fn(self, item);
    }
    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.functions += 1;
        visit::visit_impl_item_fn(self, item);
    }
    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.functions += 1;
        visit::visit_trait_item_fn(self, item);
    }
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.macros += 1;
        visit::visit_macro(self, mac);
    }
}

fn cargo_expand_available() -> bool {
    // `cargo expand --version` returns 0 iff the subcommand is installed.
    // 5-second cap keeps the probe from hanging on a wedged cargo.
    let Ok(mut child) = Command::new("cargo")
        .args(["expand", "--version"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    match wait_for(&mut child, Duration::from_secs(5)) {
        Some(status) => status.success(),
        None => false,
    }
}

fn wait_for(child: &mut std::process::Child, dur: Duration) -> Option<std::process::ExitStatus> {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        if start.elapsed() >= dur {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn count_tokens(stream: TokenStream) -> u64 {
    let mut n = 0u64;
    for tt in stream {
        n += 1;
        if let proc_macro2::TokenTree::Group(g) = tt {
            n += count_tokens(g.stream());
        }
    }
    n
}

fn posix(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn ratio(n: u64, d: u64) -> f64 {
    if d == 0 { 0.0 } else { n as f64 / d as f64 }
}
