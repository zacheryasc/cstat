//! Spec §2.8 levels 41–44: cargo workspace metadata, Cargo.lock,
//! active cfg/feature set, and toolchain pin.
//!
//! These are deliberately "F"-cost: no cargo build, no network. The
//! workspace and lockfile views come from direct TOML parsing of
//! `Cargo.toml` / `Cargo.lock`; the active-cfg view shells out to
//! `rustc --print=cfg` from inside the project root so any
//! `rust-toolchain.toml` pin is honoured by rustup; the toolchain-pin
//! view reads `rust-toolchain.toml` / `rust-toolchain` directly.

use std::process::Command;

use cstat_orchestrate::{Context, CostClass, Level, LevelError, LevelOutput};

// ───── Level 41: cargo workspace metadata ───────────────────────────────

pub(crate) struct CargoWorkspaceLevel;

impl Level for CargoWorkspaceLevel {
    fn id(&self) -> &'static str { "cargo.workspace" }
    fn cost_class(&self) -> CostClass { CostClass::F }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let manifest = ctx.project_root.join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest)
            .map_err(|e| LevelError::Failed(format!("read Cargo.toml: {e}")))?;
        let parsed: toml::Value = text
            .parse()
            .map_err(|e| LevelError::Failed(format!("parse Cargo.toml: {e}")))?;
        let table = parsed
            .as_table()
            .ok_or_else(|| LevelError::Failed("Cargo.toml root is not a table".into()))?;

        // Member count: virtual workspaces use [workspace.members]; a
        // single-package crate counts as one member; a workspace root that
        // is also a package counts the package plus any explicit members.
        let workspace_members = table
            .get("workspace")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("members"))
            .and_then(|v| v.as_array())
            .map(|a| a.len() as u64)
            .unwrap_or(0);
        let has_package = table.contains_key("package");
        let has_workspace = table.contains_key("workspace");
        let member_count = match (has_workspace, has_package) {
            (true, true) => workspace_members + 1,
            (true, false) => workspace_members,
            (false, true) => 1,
            (false, false) => 0,
        };

        let count_table = |key: &str| -> u64 {
            table
                .get(key)
                .and_then(|v| v.as_table())
                .map(|t| t.len() as u64)
                .unwrap_or(0)
        };
        let direct_deps = count_table("dependencies");
        let dev_deps = count_table("dev-dependencies");
        let build_deps = count_table("build-dependencies");
        let workspace_deps = table
            .get("workspace")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("dependencies"))
            .and_then(|v| v.as_table())
            .map(|t| t.len() as u64)
            .unwrap_or(0);
        let feature_count = count_table("features");

        let edition = table
            .get("package")
            .and_then(|p| p.get("edition"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let has_rust_version = table
            .get("package")
            .and_then(|p| p.get("rust-version"))
            .is_some();
        let is_virtual_workspace = has_workspace && !has_package;

        let vector = vec![
            ("cargo.workspace.member_count".into(), member_count as f64),
            ("cargo.workspace.direct_deps".into(), direct_deps as f64),
            ("cargo.workspace.dev_deps".into(), dev_deps as f64),
            ("cargo.workspace.build_deps".into(), build_deps as f64),
            ("cargo.workspace.workspace_deps".into(), workspace_deps as f64),
            ("cargo.workspace.feature_count".into(), feature_count as f64),
            ("cargo.workspace.edition".into(), edition as f64),
            ("cargo.workspace.has_rust_version".into(), bool_f64(has_rust_version)),
            ("cargo.workspace.is_virtual".into(), bool_f64(is_virtual_workspace)),
        ];

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Level 42: Cargo.lock ─────────────────────────────────────────────

pub(crate) struct CargoLockLevel;

impl Level for CargoLockLevel {
    fn id(&self) -> &'static str { "cargo.lock" }
    fn cost_class(&self) -> CostClass { CostClass::F }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let lock = ctx.project_root.join("Cargo.lock");
        if !lock.is_file() {
            return Err(LevelError::Skipped(
                "no Cargo.lock at project root (lockfile not generated)".into(),
            ));
        }
        let text = std::fs::read_to_string(&lock)
            .map_err(|e| LevelError::Failed(format!("read Cargo.lock: {e}")))?;
        let parsed: toml::Value = text
            .parse()
            .map_err(|e| LevelError::Failed(format!("parse Cargo.lock: {e}")))?;
        let table = parsed
            .as_table()
            .ok_or_else(|| LevelError::Failed("Cargo.lock root is not a table".into()))?;

        let lockfile_version = table
            .get("version")
            .and_then(|v| v.as_integer())
            .unwrap_or(0) as f64;

        let packages = table
            .get("package")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);

        // Dep count per package — the only signal we can derive without a
        // full graph walk. Aggregate as max + mean; downstream consumers can
        // bucket from there. "Lockfile depth" in spec §2.8 #42 is the
        // longest chain in the resolved graph; computing it properly waits
        // for Stage 9's graph derivations, where the same graph supports
        // multiple consumers. The per-package fan-out captured here is the
        // stable scalar we surface today.
        let mut dep_counts: Vec<u64> = Vec::with_capacity(packages.len());
        for pkg in packages {
            let deps = pkg
                .get("dependencies")
                .and_then(|v| v.as_array())
                .map(|a| a.len() as u64)
                .unwrap_or(0);
            dep_counts.push(deps);
        }

        let max_deps = dep_counts.iter().copied().max().unwrap_or(0);
        let total_deps: u64 = dep_counts.iter().sum();
        let mean_deps = if dep_counts.is_empty() {
            0.0
        } else {
            total_deps as f64 / dep_counts.len() as f64
        };

        let vector = vec![
            ("cargo.lock.package_count".into(), packages.len() as f64),
            ("cargo.lock.lockfile_version".into(), lockfile_version),
            ("cargo.lock.dep_edge_count".into(), total_deps as f64),
            ("cargo.lock.dep_count_per_package.max".into(), max_deps as f64),
            ("cargo.lock.dep_count_per_package.mean".into(), mean_deps),
        ];

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Level 43: active cfg / feature set ───────────────────────────────

pub(crate) struct CargoCfgLevel;

impl Level for CargoCfgLevel {
    fn id(&self) -> &'static str { "cargo.cfg" }
    fn cost_class(&self) -> CostClass { CostClass::F }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let output = Command::new("rustc")
            .arg("--print=cfg")
            .current_dir(&ctx.project_root)
            .output()
            .map_err(|e| LevelError::Skipped(format!("rustc not available: {e}")))?;
        if !output.status.success() {
            return Err(LevelError::Failed(format!(
                "rustc --print=cfg exited {:?}: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);

        let mut entry_count = 0u64;
        let mut target_feature_count = 0u64;
        let mut family_unix = false;
        let mut family_windows = false;
        let mut has_debug_assertions = false;
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            entry_count += 1;
            if let Some(rest) = line.strip_prefix("target_family=") {
                let v = rest.trim_matches('"');
                if v == "unix" {
                    family_unix = true;
                }
                if v == "windows" {
                    family_windows = true;
                }
            } else if line == "unix" {
                family_unix = true;
            } else if line == "windows" {
                family_windows = true;
            } else if line == "debug_assertions" {
                has_debug_assertions = true;
            } else if line.starts_with("target_feature=") {
                target_feature_count += 1;
            }
        }

        let vector = vec![
            ("cargo.cfg.entry_count".into(), entry_count as f64),
            ("cargo.cfg.target_feature_count".into(), target_feature_count as f64),
            ("cargo.cfg.family_unix".into(), bool_f64(family_unix)),
            ("cargo.cfg.family_windows".into(), bool_f64(family_windows)),
            (
                "cargo.cfg.debug_assertions".into(),
                bool_f64(has_debug_assertions),
            ),
        ];

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Level 44: toolchain pin ──────────────────────────────────────────

pub(crate) struct CargoToolchainLevel;

impl Level for CargoToolchainLevel {
    fn id(&self) -> &'static str { "cargo.toolchain" }
    fn cost_class(&self) -> CostClass { CostClass::F }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        // Modern form: rust-toolchain.toml. Legacy form: a plain `rust-toolchain`
        // file whose contents are just the channel name. Either pins the toolchain.
        let toml_path = ctx.project_root.join("rust-toolchain.toml");
        let plain_path = ctx.project_root.join("rust-toolchain");

        let mut pinned = false;
        let mut has_channel = false;
        let mut component_count = 0u64;
        let mut target_count = 0u64;
        let mut profile_set = false;
        let mut is_modern_form = false;

        if toml_path.is_file() {
            pinned = true;
            is_modern_form = true;
            let text = std::fs::read_to_string(&toml_path)
                .map_err(|e| LevelError::Failed(format!("read rust-toolchain.toml: {e}")))?;
            let parsed: toml::Value = text
                .parse()
                .map_err(|e| LevelError::Failed(format!("parse rust-toolchain.toml: {e}")))?;
            if let Some(tc) = parsed.get("toolchain").and_then(|v| v.as_table()) {
                has_channel = tc.contains_key("channel");
                profile_set = tc.contains_key("profile");
                if let Some(arr) = tc.get("components").and_then(|v| v.as_array()) {
                    component_count = arr.len() as u64;
                }
                if let Some(arr) = tc.get("targets").and_then(|v| v.as_array()) {
                    target_count = arr.len() as u64;
                }
            }
        } else if plain_path.is_file() {
            pinned = true;
            // Legacy plain-text form: the file is the channel name itself.
            let text = std::fs::read_to_string(&plain_path).unwrap_or_default();
            has_channel = !text.trim().is_empty();
        }

        let vector = vec![
            ("cargo.toolchain.pinned".into(), bool_f64(pinned)),
            ("cargo.toolchain.is_modern_form".into(), bool_f64(is_modern_form)),
            ("cargo.toolchain.has_channel".into(), bool_f64(has_channel)),
            ("cargo.toolchain.component_count".into(), component_count as f64),
            ("cargo.toolchain.target_count".into(), target_count as f64),
            ("cargo.toolchain.has_profile".into(), bool_f64(profile_set)),
        ];

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

fn bool_f64(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}
