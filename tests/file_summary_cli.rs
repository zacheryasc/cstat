use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_project(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("cstat-file-summary-{name}-{unique}"));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src/lib.rs"),
        r#"
mod other;

use crate::other::external_helper;
use std::fmt::Debug;

const LIMIT: usize = 10;
static NAME: &str = "fixture";

struct Parser;
enum Mode {
    Fast,
    Slow,
}

trait Parse {
    fn parse(&self);
}

fn live() {
    helper();
    external_helper();
}

fn helper() {}

fn orphan() {}

fn dead_parent() {
    dead_child();
}

fn dead_child() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_live() {
        live();
    }
}
"#,
    )
    .unwrap();
    fs::write(
        root.join("src/other.rs"),
        r#"
pub fn external_helper() {}
pub fn other_dead() {}
"#,
    )
    .unwrap();
    root
}

fn run_cstat(path: &Path, args: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_cstat");
    let mut command = Command::new(bin);
    command.args(["--no-color", "--path"]);
    command.arg(path);
    command.args(args);
    command.output().expect("invoke cstat binary")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "cstat failed: status={:?}\nstderr={}\nstdout={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
}

#[test]
fn file_section_commands_accept_file_path() {
    let root = temp_project("sections");
    let file = root.join("src/lib.rs");
    for command in [
        "loc",
        "symbols",
        "deps",
        "dead-code",
        "test-reachability",
        "call-trace",
    ] {
        let output = run_cstat(&file, &["-v", command]);
        assert_success(&output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(!stdout.is_empty(), "empty output for {command}");
        if command == "symbols" {
            for expected in [
                "fn 6", "struct 1", "enum 1", "trait 1", "const 1", "static 1",
            ] {
                assert!(stdout.contains(expected), "missing {expected}: {stdout}");
            }
        }
        if command == "call-trace" {
            for expected in ["live", "helper", "other::external_helper"] {
                assert!(stdout.contains(expected), "missing {expected}: {stdout}");
            }
        }
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_path_errors_are_clear() {
    let root = temp_project("errors");
    let notes = root.join("notes.rs");
    fs::write(&notes, "fn scratch() {}\n").unwrap();

    let output = run_cstat(&notes, &["loc"]);
    assert!(
        !output.status.success(),
        "unexpected success: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("file is not under this crate's src, tests, or benches"),
        "stderr={stderr}",
    );

    let output = run_cstat(&notes, &["--json", "loc"]);
    assert!(
        !output.status.success(),
        "unexpected json success: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: Value = serde_json::from_str(&stdout).expect("parse error JSON");
    assert!(
        value["error"]
            .as_str()
            .unwrap()
            .contains("file is not under this crate's src, tests, or benches"),
        "json={value}",
    );

    fs::remove_dir_all(root).unwrap();
}
