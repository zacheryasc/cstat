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
    let root = std::env::temp_dir().join(format!("cstat-coverage-cli-{name}-{unique}"));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src/lib.rs"),
        r#"pub fn target(flag: bool) -> u8 {
    if flag {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_true_arm() {
        assert_eq!(target(true), 1);
    }
}
"#,
    )
    .unwrap();
    root
}

fn export_json(root: &Path) -> PathBuf {
    let source = root.join("src/lib.rs");
    let escaped_source = source.to_string_lossy().replace('\\', "\\\\");
    let content = format!(
        r#"{{"type":"llvm.coverage.json.export","data":[{{"files":[{{"filename":"{}","segments":[[1,1,1,true,true,false],[2,5,1,true,true,false],[3,9,1,true,true,false],[5,9,0,true,true,false]],"branches":[[2,8,2,12,1,0,0,0,0]],"summary":{{}}}}]}}]}}"#,
        escaped_source
    );
    let export = root.join("coverage.json");
    fs::write(&export, content).unwrap();
    export
}

fn run_cstat(path: &Path, export: &Path, args: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_cstat");
    let mut command = Command::new(bin);
    command.args(["--no-color", "--path"]);
    command.arg(path);
    command.args(args);
    command.env("CSTAT_LLVM_COV_EXPORT_JSON", export);
    command.output().expect("invoke cstat binary")
}

fn run_cstat_without_export(path: &Path, args: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_cstat");
    let mut command = Command::new(bin);
    command.args(["--no-color", "--path"]);
    command.arg(path);
    command.args(args);
    command.env_remove("CSTAT_LLVM_COV_EXPORT_JSON");
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
fn coverage_verbose_prints_guidance_before_no_run_env_error() {
    let root = temp_project("missing-env");
    let output = run_cstat_without_export(&root, &["coverage", "--no-run", "-v"]);
    assert!(
        !output.status.success(),
        "coverage --no-run without export env should fail"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Normal mode runs cargo test with Rust source-based coverage flags"),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("Use coverage --no-run with CSTAT_LLVM_COV_EXPORT_JSON"),
        "stdout={stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("CSTAT_LLVM_COV_EXPORT_JSON is required"),
        "stderr={stderr}"
    );

    fs::remove_dir_all(root).unwrap();
}
#[test]
fn coverage_no_run_reports_dynamic_lines_and_branches_from_export_json() {
    let root = temp_project("json");
    let file = root.join("src/lib.rs");
    let export = export_json(&root);
    let output = run_cstat(&file, &export, &["--json", "coverage", "--no-run"]);
    assert_success(&output);

    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: Value = serde_json::from_str(&stdout).expect("parse coverage JSON");
    assert_eq!(value["target"], "src/lib.rs");
    assert!(value.get("static_reachability").is_some(), "json={stdout}");
    assert!(
        value.get("projected_line_reachability").is_some(),
        "json={stdout}"
    );
    assert_eq!(value["dynamic_line_coverage"]["coverable_lines"], 4);
    assert_eq!(value["dynamic_line_coverage"]["executed_lines"], 3);
    assert_eq!(
        value["dynamic_line_coverage"]["missing_line_numbers"],
        serde_json::json!([5])
    );
    assert_eq!(value["branch_coverage"]["total_branch_outcomes"], 2);
    assert_eq!(value["branch_coverage"]["covered_branch_outcomes"], 1);
    assert_eq!(value["branch_coverage"]["missing_branch_outcomes"], 1);
    assert_eq!(
        value["branch_coverage"]["sites"][0]["taken"],
        serde_json::json!(["true"])
    );
    assert_eq!(
        value["branch_coverage"]["sites"][0]["missing"],
        serde_json::json!(["false"])
    );
    assert_eq!(value["tool_status"]["ran_tests"], false);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn coverage_human_separates_static_projected_dynamic_and_branch_sections() {
    let root = temp_project("human");
    let file = root.join("src/lib.rs");
    let export = export_json(&root);
    let output = run_cstat(&file, &export, &["coverage", "--no-run", "-v"]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    for section in [
        "Static test/benchmark reachability",
        "Projected line reachability",
        "Dynamic line coverage",
        "Branch coverage",
    ] {
        assert_eq!(
            stdout
                .lines()
                .filter(|line| line.trim_start().starts_with(&format!("┌─ {section}")))
                .count(),
            1,
            "section header should appear once: {section}: {stdout}"
        );
    }
    for expected in [
        "Normal mode runs cargo test with Rust source-based coverage flags",
        "Line coverage is dynamic llvm-cov segment data; branch coverage uses branch records when the Rust/LLVM toolchain emits them.",
        "Reachability is computed from the static call graph; it is not runtime coverage.",
        "Projected reachable lines are production code lines inside functions statically reached from project tests/benches.",
        "Coverable/executed lines come from llvm-cov segment counts after cstat's production-span filter.",
        "Branch coverage counts true/false outcomes from llvm-cov branch records.",
        "missing lines: 5",
        "line 2: flag missing false",
    ] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }
    assert!(!stdout.contains("Path coverage"), "stdout={stdout}");

    fs::remove_dir_all(root).unwrap();
}
