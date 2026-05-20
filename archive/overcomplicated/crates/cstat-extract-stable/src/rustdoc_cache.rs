//! Lazy, shared parse of `cargo +nightly rustdoc -- -Z unstable-options
//! --output-format json`, used by spec §2.4 levels 17 / 19 (and slated
//! to back §2.9 level 46 once Stage 10 lands).
//!
//! rustdoc JSON is nightly-only and the wire format is unstable —
//! `format_version` bumps with each nightly that changes the shape. We
//! deserialize only the fields we actually consume (impls, paths,
//! external-crate names) so unknown additions don't break us; the
//! `format_version` is recorded but not gated on, since any field-rename
//! would surface as missing-keys rather than a parse error.
//!
//! The cache runs the subprocess at most once per `cstat extract` run.
//! Without nightly installed it returns Skipped with a reason; on a
//! cargo rustdoc failure it returns Failed with the stderr tail. Levels
//! that need rustdoc translate either outcome into their own report so
//! the user sees one consistent reason across the whole rustdoc-derived
//! row.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

use cstat_orchestrate::{Context, LevelError};
use serde::Deserialize;

/// One parsed rustdoc JSON document plus the source path it came from.
#[derive(Debug, Clone)]
pub(crate) struct RustdocDoc {
    /// Retained for diagnostics in case a future level wants to point
    /// the user at the on-disk JSON for debugging.
    #[allow(dead_code)]
    pub source_path: PathBuf,
    pub data: Arc<RustdocJson>,
}

/// Top-level rustdoc JSON. We carry the fields three levels need; serde's
/// default `#[deny(unknown_fields)]` is OFF (the derive default), so
/// nightly drift that adds fields is absorbed silently.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub(crate) struct RustdocJson {
    pub format_version: u32,
    #[serde(default)]
    pub root: serde_json::Value,
    #[serde(default)]
    pub external_crates: HashMap<String, ExternalCrate>,
    #[serde(default)]
    pub index: HashMap<String, IndexItem>,
    #[serde(default)]
    pub paths: HashMap<String, ItemSummary>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub(crate) struct ExternalCrate {
    pub name: String,
}

/// A `paths` entry — every item rustdoc knows about (local and foreign)
/// has one of these. `crate_id == 0` is the crate being documented;
/// every other value points into `external_crates`.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub(crate) struct ItemSummary {
    pub crate_id: u32,
    #[serde(default)]
    pub path: Vec<String>,
    pub kind: String,
}

/// An `index` entry. We keep `inner` as a raw `serde_json::Value` because
/// rustdoc tags each variant with a different key (`"function"`,
/// `"impl"`, `"struct"`, …) and exhaustively typing every variant for
/// only the impl / trait inspection we need would be more brittle than
/// a targeted lookup.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub(crate) struct IndexItem {
    #[serde(default)]
    pub name: Option<String>,
    pub crate_id: u32,
    /// Top-level docstring on the item. Present and non-empty when the
    /// user wrote a doc comment / `#[doc = "..."]`; `None` or empty
    /// otherwise. The level-46 inventory uses this for the documented
    /// ratio.
    #[serde(default)]
    pub docs: Option<String>,
    /// `"public"`, `"default"`, `"crate"`, or `"restricted"` in current
    /// rustdoc JSON. Optional because rustdoc has historically used
    /// either a string or an object here.
    #[serde(default)]
    pub visibility: serde_json::Value,
    pub inner: serde_json::Value,
}

#[derive(Clone)]
pub(crate) struct SharedRustdoc {
    inner: Arc<OnceLock<RustdocResult>>,
}

type RustdocResult = Result<RustdocDoc, RustdocSkip>;

#[derive(Debug, Clone)]
struct RustdocSkip {
    reason: String,
    is_failure: bool,
}

impl RustdocSkip {
    fn into_level_error(self) -> LevelError {
        if self.is_failure {
            LevelError::Failed(self.reason)
        } else {
            LevelError::Skipped(self.reason)
        }
    }
    fn skipped(reason: impl Into<String>) -> Self {
        Self { reason: reason.into(), is_failure: false }
    }
    fn failed(reason: impl Into<String>) -> Self {
        Self { reason: reason.into(), is_failure: true }
    }
}

impl SharedRustdoc {
    pub(crate) fn new() -> Self {
        Self { inner: Arc::new(OnceLock::new()) }
    }

    pub(crate) fn get(&self, ctx: &Context) -> Result<RustdocDoc, LevelError> {
        let result = self.inner.get_or_init(|| build_rustdoc(ctx));
        match result {
            Ok(doc) => Ok(doc.clone()),
            Err(skip) => Err(skip.clone().into_level_error()),
        }
    }
}

fn build_rustdoc(ctx: &Context) -> RustdocResult {
    // Test/diagnostic override: `CSTAT_FORCE_NO_NIGHTLY=1` simulates a host
    // without a nightly toolchain. Anchor 7 uses this so the
    // nightly-unavailable contract is exercisable from a host that happens
    // to have nightly installed (per dev-plan §"Anchor tests" #7).
    if force_no_nightly() {
        return Err(RustdocSkip::skipped(
            "nightly toolchain unavailable (forced via CSTAT_FORCE_NO_NIGHTLY)",
        ));
    }
    // Nightly is required: rustdoc JSON is a `-Z unstable-options` feature.
    // Probe via `rustup which --toolchain nightly cargo`; absence is the
    // documented Skipped path (spec §3, §11 "tool missing").
    let nightly_probe = Command::new("rustup")
        .args(["which", "--toolchain", "nightly", "cargo"])
        .output();
    match nightly_probe {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            return Err(RustdocSkip::skipped(format!(
                "nightly toolchain unavailable for rustdoc JSON: {}",
                stderr.lines().next().unwrap_or("").trim()
            )));
        }
        Err(e) => {
            return Err(RustdocSkip::skipped(format!(
                "rustup not on PATH (cannot select nightly for rustdoc JSON): {e}"
            )));
        }
    }

    // Virtual workspaces have no single package; rustdoc takes `-p` to
    // disambiguate but v1 keeps the scope-narrowing simple — skip with
    // a documented reason. The cargo level already explains why.
    let manifest = match std::fs::read_to_string(ctx.project_root.join("Cargo.toml")) {
        Ok(t) => t,
        Err(e) => return Err(RustdocSkip::failed(format!("read Cargo.toml: {e}"))),
    };
    let parsed: toml::Value = match manifest.parse() {
        Ok(v) => v,
        Err(e) => return Err(RustdocSkip::failed(format!("parse Cargo.toml: {e}"))),
    };
    if parsed.get("package").is_none() {
        return Err(RustdocSkip::skipped(
            "rustdoc JSON on virtual workspaces is not implemented (v1 limitation)",
        ));
    }

    // Drive cargo rustdoc into a private target dir so it doesn't fight
    // the codegen builds for the same fingerprint locks. Library target
    // first — if there's no lib, retry without `--lib` for bin crates.
    let target_dir = ctx.project_root.join("target").join("cstat-rustdoc");
    let stderr = match run_cargo_rustdoc(&ctx.project_root, &target_dir) {
        Ok(()) => String::new(),
        Err(e) => return Err(RustdocSkip::failed(format!("cargo rustdoc: {e}"))),
    };
    let _ = stderr; // currently unused but kept for future diagnostics.

    // rustdoc emits `target/cstat-rustdoc/doc/<crate_name>.json`. The
    // crate name comes from the manifest; we glob the directory in case
    // multiple files are produced (workspace member docs may co-locate).
    let doc_dir = target_dir.join("doc");
    let candidates = collect_json(&doc_dir);
    let json_path = match pick_doc_file(&candidates, &parsed) {
        Some(p) => p,
        None => {
            return Err(RustdocSkip::failed(format!(
                "cargo rustdoc produced no JSON under {}",
                doc_dir.display()
            )));
        }
    };

    let bytes = match std::fs::read(&json_path) {
        Ok(b) => b,
        Err(e) => return Err(RustdocSkip::failed(format!("read {}: {e}", json_path.display()))),
    };
    let data: RustdocJson = match serde_json::from_slice(&bytes) {
        Ok(d) => d,
        Err(e) => {
            return Err(RustdocSkip::failed(format!(
                "parse rustdoc JSON at {}: {e}",
                json_path.display()
            )));
        }
    };

    Ok(RustdocDoc {
        source_path: json_path,
        data: Arc::new(data),
    })
}

fn run_cargo_rustdoc(project_root: &Path, target_dir: &Path) -> Result<(), String> {
    let attempts: [&[&str]; 2] = [&["--lib"], &[]];
    let mut last = String::new();
    for attempt in attempts {
        let mut cmd = Command::new("cargo");
        cmd.arg("+nightly").arg("rustdoc");
        for a in attempt {
            cmd.arg(a);
        }
        cmd.arg("--target-dir").arg(target_dir);
        cmd.arg("--").args(["-Z", "unstable-options", "--output-format", "json"]);
        cmd.current_dir(project_root);
        let output = cmd
            .output()
            .map_err(|e| format!("spawn cargo: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("no library targets") && !stderr.contains("no lib target") {
            return Err(format!(
                "exited {:?}: {}",
                output.status.code(),
                stderr.lines().rev().take(6).collect::<Vec<_>>().join(" | ")
            ));
        }
        last = stderr.into_owned();
    }
    Err(format!(
        "all cargo rustdoc attempts failed: {}",
        last.lines().rev().take(4).collect::<Vec<_>>().join(" | ")
    ))
}

fn collect_json(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    out.sort();
    out
}

/// `CSTAT_FORCE_NO_NIGHTLY=1` simulates a host without nightly. Used by
/// the anchor-7 nightly-unavailable contract test on hosts that happen
/// to have nightly installed.
pub(crate) fn force_no_nightly() -> bool {
    std::env::var_os("CSTAT_FORCE_NO_NIGHTLY")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false)
}

/// Pick the rustdoc JSON file that corresponds to the project's own
/// package. rustdoc writes `<crate_name>.json` (underscored); the
/// manifest holds the canonical name. If we can't see one matching
/// the manifest, fall back to whichever single file is present.
fn pick_doc_file(candidates: &[PathBuf], manifest: &toml::Value) -> Option<PathBuf> {
    let pkg_name = manifest
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let underscored = pkg_name.replace('-', "_");
    for p in candidates {
        if p.file_stem().and_then(|s| s.to_str()) == Some(underscored.as_str()) {
            return Some(p.clone());
        }
    }
    candidates.first().cloned()
}

impl RustdocJson {
    /// True when the given `paths`-key id refers to an item defined in
    /// the documented crate (crate_id == 0 per rustdoc convention).
    pub(crate) fn is_local_path(&self, id_key: &str) -> bool {
        self.paths
            .get(id_key)
            .map(|p| p.crate_id == 0)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_doc_file_prefers_manifest_name_match() {
        let manifest: toml::Value = "[package]\nname = \"foo-bar\"\n".parse().unwrap();
        let candidates = vec![
            PathBuf::from("/x/doc/other.json"),
            PathBuf::from("/x/doc/foo_bar.json"),
        ];
        assert_eq!(
            pick_doc_file(&candidates, &manifest).unwrap(),
            PathBuf::from("/x/doc/foo_bar.json")
        );
    }

    #[test]
    fn pick_doc_file_falls_back_to_first_when_no_match() {
        let manifest: toml::Value = "[package]\nname = \"baz\"\n".parse().unwrap();
        let candidates = vec![PathBuf::from("/x/doc/other.json")];
        assert_eq!(
            pick_doc_file(&candidates, &manifest).unwrap(),
            PathBuf::from("/x/doc/other.json")
        );
    }
}
