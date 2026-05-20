//! Argument parsing + translation into a synthetic rustc command line.
//!
//! The orchestrator (Stage 13) invokes this binary with `--path` and
//! `--output`. We assemble the rustc-driver args ourselves rather than
//! shelling out to cargo so the IR walk fires exactly once on the
//! requested crate root and no codegen pass runs (`--emit=metadata`).
//!
//! v1 limitation: only single-crate projects with `src/lib.rs` or
//! `src/main.rs` are recognised. Workspaces and crates with external
//! dependencies are out of scope for Stage 12 — the orchestrator can
//! grow a cargo-driven driver in a later stage.

use std::path::PathBuf;

pub struct Args {
    pub project: PathBuf,
    pub output: PathBuf,
}

pub fn parse() -> Result<Args, String> {
    let mut project: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--path" => {
                project =
                    Some(PathBuf::from(it.next().ok_or_else(|| {
                        "--path needs a value".to_string()
                    })?));
            }
            "--output" => {
                output =
                    Some(PathBuf::from(it.next().ok_or_else(|| {
                        "--output needs a value".to_string()
                    })?));
            }
            "-h" | "--help" => {
                println!("{}", help_text());
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let project = project.ok_or_else(|| "missing --path <project-root>".to_string())?;
    let output = output.ok_or_else(|| "missing --output <out.json>".to_string())?;
    if !project.is_dir() {
        return Err(format!(
            "--path is not a directory: {}",
            project.display()
        ));
    }
    Ok(Args { project, output })
}

fn help_text() -> &'static str {
    "cstat-extract-rustc --path <project-root> --output <out.json>\n\
     \n\
     Runs rustc on the given Rust project (single-crate, src/lib.rs or\n\
     src/main.rs) and writes a cstat-ir JSON dump covering HIR, THIR,\n\
     pre-/post-opt MIR, borrowck artifacts, and mono-items."
}

pub fn build_rustc_args(args: &Args, sysroot: &str) -> Result<Vec<String>, String> {
    let manifest_path = args.project.join("Cargo.toml");
    let manifest_txt = std::fs::read_to_string(&manifest_path).map_err(|e| {
        format!(
            "failed to read manifest {}: {e}",
            manifest_path.display()
        )
    })?;
    let (crate_name, edition) = parse_manifest(&manifest_txt)?;
    let canonical_name = crate_name.replace('-', "_");

    let lib_rs = args.project.join("src/lib.rs");
    let main_rs = args.project.join("src/main.rs");
    let (root, crate_type) = if lib_rs.exists() {
        (lib_rs, "lib")
    } else if main_rs.exists() {
        (main_rs, "bin")
    } else {
        return Err(format!(
            "no src/lib.rs or src/main.rs under {}",
            args.project.display()
        ));
    };

    let out_dir = std::env::temp_dir().join(format!(
        "cstat-extract-rustc-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("create temp out-dir {}: {e}", out_dir.display()))?;

    let v: Vec<String> = vec![
        "rustc".to_string(),
        root.to_string_lossy().into_owned(),
        "--crate-name".into(),
        canonical_name,
        "--edition".into(),
        edition,
        "--crate-type".into(),
        crate_type.into(),
        "--sysroot".into(),
        sysroot.into(),
        // Default emit (link) ensures mono-item collection runs. We
        // stop the driver before codegen actually writes anything, so
        // the linker doesn't get invoked even though the emit set says
        // "link". `--out-dir` keeps any partial intermediate artifacts
        // off the user's CWD.
        "--out-dir".into(),
        out_dir.to_string_lossy().into_owned(),
        "-A".into(),
        "warnings".into(),
    ];

    Ok(v)
}

fn parse_manifest(txt: &str) -> Result<(String, String), String> {
    let parsed: toml::Value = txt
        .parse()
        .map_err(|e| format!("parse Cargo.toml: {e}"))?;
    let pkg = parsed
        .get("package")
        .ok_or_else(|| "Cargo.toml missing [package] table".to_string())?;
    let name = pkg
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Cargo.toml missing package.name".to_string())?
        .to_string();
    let edition = pkg
        .get("edition")
        .and_then(|v| v.as_str())
        .unwrap_or("2021")
        .to_string();
    Ok((name, edition))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_manifest_reads_name_and_edition() {
        let txt = r#"
[package]
name = "demo-pkg"
edition = "2021"
"#;
        let (name, edition) = parse_manifest(txt).unwrap();
        assert_eq!(name, "demo-pkg");
        assert_eq!(edition, "2021");
    }

    #[test]
    fn parse_manifest_defaults_edition_when_missing() {
        let txt = r#"[package]
name = "x""#;
        let (_, edition) = parse_manifest(txt).unwrap();
        assert_eq!(edition, "2021");
    }

    #[test]
    fn parse_manifest_rejects_missing_package_table() {
        let err = parse_manifest("[workspace]\nmembers=[]\n").unwrap_err();
        assert!(err.contains("[package]"));
    }
}
