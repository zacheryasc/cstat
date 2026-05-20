//! Toolchain probe — channel/version/host for the dump's `ToolchainStamp`,
//! plus sysroot discovery used to feed `--sysroot` to `RunCompiler`.
//!
//! The sysroot is captured at build time via `build.rs` so the binary
//! always points at the same nightly that linked against `rustc_driver`.
//! Falling back to `rustc --print sysroot` at runtime would risk picking
//! up a different toolchain via PATH order, which makes `rustc_driver`
//! ABI-incompatible and panics deep inside `RunCompiler::run`.

use cstat_ir::ToolchainStamp;

const BUILD_TIME_SYSROOT: Option<&'static str> = option_env!("CSTAT_EXTRACT_RUSTC_SYSROOT");

pub fn sysroot() -> Option<String> {
    if let Some(s) = BUILD_TIME_SYSROOT {
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    let out = std::process::Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}

pub fn probe() -> ToolchainStamp {
    let version = run_rustc(&["--version"]).unwrap_or_else(|| "unknown".into());
    let mut host = "unknown".to_string();
    let mut channel = if version.contains("nightly") {
        "nightly".to_string()
    } else if version.contains("beta") {
        "beta".to_string()
    } else {
        "stable".to_string()
    };
    if let Some(vv) = run_rustc(&["-vV"]) {
        for line in vv.lines() {
            if let Some(rest) = line.strip_prefix("host: ") {
                host = rest.trim().to_string();
            }
            if let Some(rest) = line.strip_prefix("release: ") {
                if rest.contains("nightly") {
                    channel = "nightly".to_string();
                } else if rest.contains("beta") {
                    channel = "beta".to_string();
                } else if !rest.is_empty() {
                    channel = "stable".to_string();
                }
            }
        }
    }
    ToolchainStamp {
        channel,
        version: version.trim().to_string(),
        host,
    }
}

fn run_rustc(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("rustc")
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}
