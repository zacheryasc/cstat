//! Shared cache for the `cstat-extract-rustc` JSON dump (spec §2.3, §2.4
//! and Stage 13).
//!
//! All seven rustc-IR Level impls — `ast.resolved`, `hir`, `thir`,
//! `mir.preopt`, `mir.opt`, `borrowck`, `type.mono` — read from this one
//! cache. The first level whose `extract` runs builds and invokes the
//! nightly sub-binary; subsequent levels reuse the parsed [`CstatIrDump`].
//!
//! Configuration gates, in order:
//! 1. `Config.enable_rustc_internal` — when false, every IR level reports
//!    `Skipped` with a documented reason. Off by default because the
//!    nightly subprocess is cost class B and most consumers don't want
//!    to pay the latency on every run.
//! 2. The `nightly` rustup toolchain — when absent, `Skipped` with the
//!    standard "nightly toolchain unavailable" wording.
//! 3. A locatable extractor binary — either `CSTAT_EXTRACT_RUSTC_BIN`
//!    (used by integration tests to bypass the on-demand build) or a
//!    relative-path lookup into the workspace, with `cargo +nightly
//!    build` as a fallback.
//! 4. A non-virtual-workspace project — the extractor v1 takes a single
//!    crate root, so virtual workspaces `Skipped` with a reason.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

use cstat_ir::CstatIrDump;
use cstat_orchestrate::{Context, LevelError};

/// Lazy, shared parse of the nightly sub-binary's output.
#[derive(Clone)]
pub(crate) struct SharedIrDump {
    inner: Arc<OnceLock<IrDumpResult>>,
}

type IrDumpResult = Result<Arc<CstatIrDump>, IrDumpSkip>;

#[derive(Debug, Clone)]
struct IrDumpSkip {
    reason: String,
    is_failure: bool,
}

impl IrDumpSkip {
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

impl SharedIrDump {
    pub(crate) fn new() -> Self {
        Self { inner: Arc::new(OnceLock::new()) }
    }

    pub(crate) fn get(&self, ctx: &Context) -> Result<Arc<CstatIrDump>, LevelError> {
        let result = self.inner.get_or_init(|| build_ir_dump(ctx));
        match result {
            Ok(d) => Ok(d.clone()),
            Err(skip) => Err(skip.clone().into_level_error()),
        }
    }
}

fn build_ir_dump(ctx: &Context) -> IrDumpResult {
    if !ctx.config.enable_rustc_internal {
        return Err(IrDumpSkip::skipped(
            "rustc-internal extraction disabled (pass --enable-rustc-internal to opt in; \
             cost class B, requires nightly)",
        ));
    }

    // Probe rustup for the nightly toolchain. Same wording as the
    // rustdoc cache so a host without nightly produces a consistent
    // reason across every nightly-derived level.
    let probe = Command::new("rustup")
        .args(["which", "--toolchain", "nightly", "cargo"])
        .output();
    match probe {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            return Err(IrDumpSkip::skipped(format!(
                "nightly toolchain unavailable: {}",
                stderr.lines().next().unwrap_or("").trim()
            )));
        }
        Err(e) => {
            return Err(IrDumpSkip::skipped(format!(
                "rustup not on PATH (cannot select nightly): {e}"
            )));
        }
    }

    // Virtual workspaces: the v1 extractor takes a single crate root,
    // so skip them cleanly the way the codegen and rustdoc caches do.
    let manifest_text = match std::fs::read_to_string(ctx.project_root.join("Cargo.toml")) {
        Ok(t) => t,
        Err(e) => return Err(IrDumpSkip::failed(format!("read Cargo.toml: {e}"))),
    };
    let manifest: toml::Value = match manifest_text.parse() {
        Ok(v) => v,
        Err(e) => return Err(IrDumpSkip::failed(format!("parse Cargo.toml: {e}"))),
    };
    if manifest.get("package").is_none() {
        return Err(IrDumpSkip::skipped(
            "rustc-internal extraction on virtual workspaces is not implemented \
             (v1 limitation; invoke cstat from inside a member crate)",
        ));
    }

    // Locate (or build) the nightly sub-binary.
    let bin = match locate_extractor_binary() {
        Ok(p) => p,
        Err(reason) => return Err(IrDumpSkip::skipped(reason)),
    };

    // Compute the nightly sysroot so we can preload librustc_driver*.so
    // for the spawned process. Without this the loader can't find
    // librustc_driver-*.so and the binary aborts before main runs.
    let sysroot = match nightly_sysroot() {
        Ok(s) => s,
        Err(e) => return Err(IrDumpSkip::skipped(format!("nightly sysroot probe: {e}"))),
    };

    // Run the binary, dump JSON to a temp file, parse.
    let out_path = std::env::temp_dir().join(format!(
        "cstat-extract-rustc-stable-{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_path);

    let lib_dir = format!("{sysroot}/lib");
    let ld_path = match std::env::var_os("LD_LIBRARY_PATH") {
        Some(existing) => {
            let mut v = std::ffi::OsString::from(&lib_dir);
            v.push(":");
            v.push(existing);
            v
        }
        None => std::ffi::OsString::from(&lib_dir),
    };

    let status = Command::new(&bin)
        .arg("--path")
        .arg(&ctx.project_root)
        .arg("--output")
        .arg(&out_path)
        .env("LD_LIBRARY_PATH", ld_path)
        .output();
    let output = match status {
        Ok(o) => o,
        Err(e) => {
            return Err(IrDumpSkip::failed(format!(
                "spawn cstat-extract-rustc ({}): {e}",
                bin.display()
            )));
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(IrDumpSkip::failed(format!(
            "cstat-extract-rustc exit {:?}: {}",
            output.status.code(),
            stderr.lines().rev().take(4).collect::<Vec<_>>().join(" | ")
        )));
    }

    let bytes = match std::fs::read(&out_path) {
        Ok(b) => b,
        Err(e) => {
            return Err(IrDumpSkip::failed(format!(
                "read extractor output {}: {e}",
                out_path.display()
            )));
        }
    };
    let dump: CstatIrDump = match serde_json::from_slice(&bytes) {
        Ok(d) => d,
        Err(e) => {
            return Err(IrDumpSkip::failed(format!(
                "parse extractor JSON at {}: {e}",
                out_path.display()
            )));
        }
    };
    // Best-effort tidy; failure to delete isn't fatal.
    let _ = std::fs::remove_file(&out_path);

    Ok(Arc::new(dump))
}

/// Find the compiled nightly sub-binary, building it on demand if necessary.
///
/// Resolution order:
/// 1. `CSTAT_EXTRACT_RUSTC_BIN` env var (integration tests pre-build the
///    binary and point us at it directly).
/// 2. A pre-built artifact at the expected `target/debug` path inside
///    the `cstat-extract-rustc` crate.
/// 3. Run `cargo +nightly build --manifest-path …/cstat-extract-rustc/Cargo.toml`
///    and re-check.
fn locate_extractor_binary() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CSTAT_EXTRACT_RUSTC_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "CSTAT_EXTRACT_RUSTC_BIN points at non-existent path: {}",
            path.display()
        ));
    }

    let extractor_crate = match infer_extractor_crate_dir() {
        Some(p) => p,
        None => {
            return Err(
                "cstat-extract-rustc crate directory not located \
                 (set CSTAT_EXTRACT_RUSTC_BIN to the compiled binary path)"
                    .to_string(),
            );
        }
    };

    let bin_name = if cfg!(windows) {
        "cstat-extract-rustc.exe"
    } else {
        "cstat-extract-rustc"
    };
    let target_bin = extractor_crate
        .join("target")
        .join("debug")
        .join(bin_name);
    if target_bin.is_file() {
        return Ok(target_bin);
    }

    // Build on demand. Cargo will honour the crate-local
    // rust-toolchain.toml that pins nightly.
    let manifest = extractor_crate.join("Cargo.toml");
    let output = Command::new("cargo")
        .arg("+nightly")
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .map_err(|e| format!("spawn cargo +nightly build: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "cargo +nightly build of cstat-extract-rustc failed: {}",
            stderr.lines().rev().take(4).collect::<Vec<_>>().join(" | ")
        ));
    }

    if target_bin.is_file() {
        Ok(target_bin)
    } else {
        Err(format!(
            "nightly build of cstat-extract-rustc produced no binary at {}",
            target_bin.display()
        ))
    }
}

/// Best-effort locate of the `crates/cstat-extract-rustc` directory.
///
/// At build time, `CARGO_MANIFEST_DIR` for this crate points at
/// `crates/cstat-extract-stable`. The sibling crate lives one level up.
/// After `cargo install` this path no longer exists on disk; the caller
/// is expected to set `CSTAT_EXTRACT_RUSTC_BIN` in that case.
fn infer_extractor_crate_dir() -> Option<PathBuf> {
    let manifest_dir = option_env!("CARGO_MANIFEST_DIR")?;
    let parent = Path::new(manifest_dir).parent()?;
    let sibling = parent.join("cstat-extract-rustc");
    if sibling.is_dir() {
        Some(sibling)
    } else {
        None
    }
}

fn nightly_sysroot() -> Result<String, String> {
    let output = Command::new("rustc")
        .arg("+nightly")
        .arg("--print")
        .arg("sysroot")
        .output()
        .map_err(|e| format!("rustc +nightly --print sysroot: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "rustc +nightly --print sysroot exit {:?}: {}",
            output.status.code(),
            stderr.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
