use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const ANALYSIS_NOTE: &str = "Static test/benchmark reachability. Counts are how many test or benchmark entry roots can reach a function/edge, not runtime coverage or hit counts.";

fn temp_project(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("cstat-test-reachability-{name}-{unique}"));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src/lib.rs"),
        r#"
fn target() {
    helper();
}

fn helper() {}

fn untested() {}

mod tests {
    #[test]
    fn covers_target() {
        target();
    }
}
"#,
    )
    .unwrap();
    root
}

fn run_cstat(project: &Path, args: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_cstat");
    let mut command = Command::new(bin);
    command.args(["--no-color", "--path"]);
    command.arg(project);
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
fn test_reachability_human_output_uses_static_reachability_terms() {
    let root = temp_project("human");
    let output = run_cstat(&root, &["test-reachability"]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Static test/benchmark reachability"),
        "stdout={stdout}"
    );
    assert!(stdout.contains("test/bench roots 1"), "stdout={stdout}");
    assert!(
        stdout.contains("statically reachable 3 / 4 (75.0%)"),
        "stdout={stdout}"
    );
    assert!(stdout.contains("unreachable 1"), "stdout={stdout}");
    assert!(
        stdout.contains("Not statically reachable from tests/benches:"),
        "stdout={stdout}"
    );
    assert!(stdout.contains("untested"), "stdout={stdout}");
    for unexpected in [
        "datapath",
        "datapaths",
        "Execution Flow Heatmap",
        "random walks",
    ] {
        assert!(
            !stdout.contains(unexpected),
            "stdout unexpectedly contained {unexpected:?}: {stdout}"
        );
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_reachability_json_uses_reachability_fields() {
    let root = temp_project("json");
    let output = run_cstat(&root, &["--json", "test-reachability"]);
    assert_success(&output);

    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: Value = serde_json::from_str(&stdout).expect("parse reachability JSON");
    assert_eq!(value["test_benchmark_entry_count"], 1);
    assert_eq!(value["reachable_function_count"], 3);
    assert_eq!(value["unreachable_function_count"], 1);
    assert_eq!(value["entries"][0]["entry"], "tests::covers_target");
    assert_eq!(value["entries"][0]["kind"], "test");
    assert_eq!(value["entries"][0]["reachable_functions"], 3);
    assert_eq!(value["entries"][0]["unique_call_steps"], 2);
    assert_eq!(value["entries"][0]["max_call_depth"], 2);

    let functions = value["function_reachability_counts"]
        .as_array()
        .expect("function reachability array");
    let helper = functions
        .iter()
        .find(|entry| entry["function"] == "helper")
        .expect("helper reachability");
    assert_eq!(helper["reachable_from_count"], 1);
    assert_eq!(
        helper["reachable_from_entries"],
        serde_json::json!(["tests::covers_target"])
    );

    let edges = value["edge_reachability_counts"]
        .as_array()
        .expect("edge reachability array");
    let helper_edge = edges
        .iter()
        .find(|edge| edge["caller"] == "target" && edge["callee"] == "helper")
        .expect("target-to-helper reachability");
    assert_eq!(helper_edge["reachable_from_count"], 1);

    let unreachable = value["unreachable_functions"]
        .as_array()
        .expect("unreachable function array");
    assert!(unreachable.iter().any(|function| function == "untested"));
    assert_eq!(value["analysis_note"], ANALYSIS_NOTE);

    for old_key in [
        "function_path_counts",
        "edge_path_counts",
        "covered_function_count",
        "uncovered_function_count",
        "uncovered_functions",
        "visit_counts",
        "hot_paths",
    ] {
        assert!(
            value.get(old_key).is_none(),
            "unexpected old key: {old_key}"
        );
    }
    assert!(!stdout.contains("datapath"), "json={stdout}");
    assert!(!stdout.contains("datapaths"), "json={stdout}");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn focused_report_json_renames_top_level_field() {
    let root = temp_project("report-json");
    let output = run_cstat(&root, &["--json", "report"]);
    assert_success(&output);

    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: Value = serde_json::from_str(&stdout).expect("parse focused report JSON");
    assert!(value.get("test_reachability").is_some());
    assert!(value.get("datapaths").is_none());
    assert_eq!(value["test_reachability"]["reachable_function_count"], 3);
    assert_eq!(value["test_reachability"]["unreachable_function_count"], 1);
    for field in ["line_counts", "symbols", "dependencies", "dead_code"] {
        assert!(
            value.get(field).is_some(),
            "missing focused report field: {field}"
        );
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn advanced_flow_heatmap_runs_legacy_random_walk_analysis() {
    let root = temp_project("advanced");
    let output = run_cstat(&root, &["advanced", "flow-heatmap", "--walks", "10"]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Execution Flow Heatmap"), "stdout={stdout}");
    assert!(stdout.contains("random walks"), "stdout={stdout}");
    assert!(
        !stdout.contains("Static test/benchmark reachability"),
        "stdout={stdout}"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn old_focused_datapaths_command_is_not_available() {
    let root = temp_project("old-command");
    let output = run_cstat(&root, &["datapaths"]);
    assert!(
        !output.status.success(),
        "legacy root command unexpectedly succeeded: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn old_advanced_datapaths_command_is_not_available() {
    let root = temp_project("old-advanced-command");
    let output = run_cstat(&root, &["advanced", "datapaths"]);
    assert!(
        !output.status.success(),
        "legacy advanced command unexpectedly succeeded: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn help_lists_test_reachability_and_advanced_flow_heatmap() {
    let root = temp_project("help");

    let root_help = run_cstat(&root, &["--help"]);
    assert_success(&root_help);
    let root_stdout = String::from_utf8_lossy(&root_help.stdout);
    assert!(
        root_stdout.contains("test-reachability"),
        "stdout={root_stdout}"
    );
    assert!(
        root_stdout.contains("Static test/benchmark reachability"),
        "stdout={root_stdout}"
    );
    assert!(
        !root_stdout
            .lines()
            .any(|line| line.starts_with("  datapaths")),
        "root help still listed datapaths: {root_stdout}"
    );

    let advanced_help = run_cstat(&root, &["advanced", "--help"]);
    assert_success(&advanced_help);
    let advanced_stdout = String::from_utf8_lossy(&advanced_help.stdout);
    assert!(
        advanced_stdout.contains("flow-heatmap"),
        "stdout={advanced_stdout}"
    );
    assert!(
        !advanced_stdout
            .lines()
            .any(|line| line.starts_with("  datapaths")),
        "advanced help still listed datapaths: {advanced_stdout}"
    );
    assert!(
        advanced_stdout.contains("random-walk") || advanced_stdout.contains("heatmap"),
        "stdout={advanced_stdout}"
    );

    fs::remove_dir_all(root).unwrap();
}
