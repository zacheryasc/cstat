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

fn assert_json_array_contains(array: &Value, expected: &str) {
    let values = array.as_array().expect("json array");
    assert!(
        values.iter().any(|value| value == expected),
        "expected {expected:?} in {values:?}",
    );
}

#[test]
fn file_summary_human_accepts_file_path() {
    let root = temp_project("human");
    let file = root.join("src/lib.rs");
    let output = run_cstat(&file, &["-v", "summary"]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cstat file summary (src/lib.rs)"),
        "stdout={stdout}"
    );
    for section in [
        "Projected line reachability",
        "Static test/benchmark reachability",
        "Symbols",
        "Dependencies",
        "Dead code candidates",
        "Call trace size",
    ] {
        assert!(
            stdout.contains(section),
            "missing section {section}: {stdout}"
        );
    }
    for full_section in [
        "ℹ Full section: cstat loc --path src/lib.rs -v",
        "ℹ Full section: cstat test-reachability --path src/lib.rs -v",
        "ℹ Full section: cstat symbols --path src/lib.rs -v",
        "ℹ Full section: cstat deps --path src/lib.rs -v",
        "ℹ Full section: cstat dead-code --path src/lib.rs -v",
        "ℹ Full section: cstat call-trace --path src/lib.rs -v",
    ] {
        assert!(
            stdout.contains(full_section),
            "missing verbose line {full_section}: {stdout}",
        );
    }
    for expected in [
        "fn 6", "struct 1", "enum 1", "trait 1", "const 1", "static 1",
    ] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }
    assert!(stdout.contains("test/bench roots 1"), "stdout={stdout}");
    assert!(
        stdout.contains("not statically reachable:"),
        "stdout={stdout}"
    );
    assert!(stdout.contains("orphan"), "stdout={stdout}");
    assert!(stdout.contains("dead_child"), "stdout={stdout}");
    assert!(stdout.contains("in-file fan-in"), "stdout={stdout}");
    for expected in ["Call trace size", "live", "dead_parent", "orphan"] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_summary_json_is_untruncated_and_file_scoped() {
    let root = temp_project("json");
    let file = root.join("src/lib.rs");
    let output = run_cstat(&file, &["--json", "summary"]);
    assert_success(&output);

    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: Value = serde_json::from_str(&stdout).expect("parse file summary JSON");
    assert_eq!(value["file"], "src/lib.rs");
    assert_eq!(value["project"], "fixture");
    for field in [
        "line_counts",
        "static_reachability",
        "symbols",
        "dependencies",
        "dead_code",
        "call_trace",
    ] {
        assert!(value.get(field).is_some(), "missing {field}: {stdout}");
    }
    assert_eq!(value["symbols"]["functions"], 6);
    assert_eq!(value["symbols"]["structs"], 1);
    assert_eq!(value["symbols"]["enums"], 1);
    assert_eq!(value["symbols"]["traits"], 1);
    assert_eq!(value["symbols"]["consts"], 1);
    assert_eq!(value["symbols"]["statics"], 1);
    assert_eq!(
        value["static_reachability"]["test_benchmark_entry_count"],
        1
    );
    assert_json_array_contains(&value["static_reachability"]["reachable_functions"], "live");
    assert_json_array_contains(
        &value["static_reachability"]["reachable_functions"],
        "helper",
    );
    for function in ["orphan", "dead_parent", "dead_child"] {
        assert_json_array_contains(
            &value["static_reachability"]["unreachable_functions"],
            function,
        );
    }

    let dead_code = value["dead_code"]["functions"]
        .as_array()
        .expect("dead code functions");
    let dead_child = dead_code
        .iter()
        .find(|entry| entry["function"] == "dead_child")
        .expect("dead_child dead-code row");
    assert_eq!(dead_child["candidate"], true);
    assert_eq!(dead_child["in_file_fan_in"], 1);
    assert!(
        dead_child["reason"]
            .as_str()
            .unwrap()
            .contains("no test/benchmark root reaches that caller"),
        "dead_child reason={:?}",
        dead_child["reason"]
    );
    let orphan = dead_code
        .iter()
        .find(|entry| entry["function"] == "orphan")
        .expect("orphan dead-code row");
    assert!(
        orphan["reason"]
            .as_str()
            .unwrap()
            .contains("no direct, trait, drop, function-item, or closure edge"),
        "orphan reason={:?}",
        orphan["reason"]
    );

    let traces = value["call_trace"]["functions"]
        .as_array()
        .expect("call trace functions");
    let live = traces
        .iter()
        .find(|entry| entry["function"] == "live")
        .expect("live trace row");
    assert!(
        live["functions_reached"].as_u64().unwrap() > 1,
        "live trace={live:?}",
    );
    let orphan = traces
        .iter()
        .find(|entry| entry["function"] == "orphan")
        .expect("orphan trace row");
    assert_eq!(orphan["functions_reached"], 1);

    fs::remove_dir_all(root).unwrap();
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

    let output = run_cstat(&notes, &["summary"]);
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

    let output = run_cstat(&notes, &["--json", "summary"]);
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
