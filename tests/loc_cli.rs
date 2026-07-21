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
    let root = std::env::temp_dir().join(format!("cstat-loc-cli-{name}-{unique}"));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src/lib.rs"),
        r#"
pub fn live() {
    helper();
}

/*
block comment only
*/
fn helper() {}

/* single-line block comment only */
fn orphan() {
    // comment-only
}

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
    root
}

fn run_cstat_raw(args: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_cstat");
    let mut command = Command::new(bin);
    command.args(args);
    command.output().expect("invoke cstat binary")
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

fn span_array_contains_function(array: &Value, expected: &str) -> bool {
    array
        .as_array()
        .expect("span array")
        .iter()
        .any(|item| item.get("function").and_then(Value::as_str) == Some(expected))
}

#[test]
fn loc_help_exposes_project_and_selected_file_modes() {
    let output = run_cstat_raw(&["loc", "--help"]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "Rust project directory or Rust source file",
        "Project mode:",
        "Selected-file mode:",
        "cstat loc --path .",
        "cstat loc --path src/lib.rs",
        "cstat loc --explain",
        "cstat loc --explain --json",
        "code_lines is a static textual classification",
        "blank lines are excluded",
        "// comment-only lines are excluded",
        "block-comment-only regions are excluded",
        "Project JSON fields:",
        "Selected-file JSON fields:",
        "--top only limits the project-mode",
        "ignored for selected-file and --explain modes",
    ] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }
}

#[test]
fn loc_explain_human_is_canonical_reference() {
    let output = run_cstat_raw(&[
        "--path",
        "/definitely/missing/cstat/path",
        "loc",
        "--explain",
    ]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "loc reference",
        "Project mode: cstat loc --path .",
        "Machine contract: cstat loc --explain --json",
        "code_lines: static textual classification",
        "not semantic Rust parsing",
        "Project JSON fields:",
        "Selected-file JSON fields:",
    ] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Error discovering files"),
        "unexpected discovery error: {stderr}",
    );
}

#[test]
fn loc_explain_json_exposes_machine_contract() {
    let output = run_cstat_raw(&["--json", "loc", "--explain"]);
    assert_success(&output);

    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: Value = serde_json::from_str(&stdout).expect("parse loc explain JSON");
    assert_eq!(value["probe"], "loc");
    assert_eq!(
        value["commands"]["explain_json"],
        "cstat loc --explain --json",
    );
    assert_eq!(value["code_lines"]["kind"], "static textual classification",);
    assert!(
        value["project_json_fields"]["files"]
            .as_array()
            .expect("project fields")
            .iter()
            .any(|field| field.as_str() == Some("code_lines")),
        "json={stdout}",
    );
    assert!(
        value["selected_file_json_fields"]["reachable_spans"]
            .as_array()
            .expect("selected fields")
            .iter()
            .any(|field| field.as_str() == Some("function")),
        "json={stdout}",
    );
}

#[test]
fn loc_verbose_points_to_loc_explain_not_guide() {
    let root = temp_project("verbose-loc-reference");
    let output = run_cstat(&root, &["-v", "loc"]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Full loc reference: cstat loc --explain"),
        "stdout={stdout}",
    );
    assert!(
        !stdout.contains("cstat guide size"),
        "stdout unexpectedly referenced guide: {stdout}",
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn loc_project_json_exposes_shape_contract() {
    let root = temp_project("project-json");
    let output = run_cstat(&root, &["--json", "loc"]);
    assert_success(&output);

    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: Value = serde_json::from_str(&stdout).expect("parse loc JSON");
    assert_eq!(value["cstat_version"], env!("CARGO_PKG_VERSION"));

    let files = value["files"].as_array().expect("files array");
    assert!(!files.is_empty(), "json={stdout}");
    let lib = files
        .iter()
        .find(|file| file["path"] == "src/lib.rs")
        .expect("src/lib.rs row");
    let total_lines = lib["total_lines"].as_u64().expect("total_lines");
    let code_lines = lib["code_lines"].as_u64().expect("code_lines");
    assert!(
        total_lines >= code_lines && code_lines > 0,
        "lib row={lib:?}",
    );

    let aggregate = &value["aggregate"];
    for field in [
        "total_files",
        "total_loc",
        "mean",
        "std_dev",
        "median",
        "min",
        "max",
    ] {
        assert!(aggregate.get(field).is_some(), "missing {field}: {stdout}");
    }

    let directories = value["directory_breakdown"]
        .as_array()
        .expect("directory_breakdown array");
    assert!(
        directories.iter().any(|directory| {
            directory["directory"] == "src"
                && directory["code_lines"]
                    .as_u64()
                    .is_some_and(|code_lines| code_lines > 0)
        }),
        "directory_breakdown={directories:?}",
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn loc_selected_file_human_is_usable() {
    let root = temp_project("selected-human");
    let file = root.join("src/lib.rs");
    let output = run_cstat(&file, &["loc"]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "Projected line reachability",
        "file src/lib.rs",
        "physical lines",
        "production code lines",
        "statically reachable lines",
        "not statically reachable lines",
        "reachable function spans:",
        "unreachable function spans:",
        "live",
        "orphan",
    ] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn loc_selected_file_json_exposes_projected_reachability_contract() {
    let root = temp_project("selected-json");
    let file = root.join("src/lib.rs");
    let output = run_cstat(&file, &["--json", "loc"]);
    assert_success(&output);

    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: Value = serde_json::from_str(&stdout).expect("parse selected-file loc JSON");
    assert_eq!(value["file"], "src/lib.rs");
    let total_lines = value["total_lines"].as_u64().expect("total_lines");
    let code_lines = value["code_lines"].as_u64().expect("code_lines");
    let projected_reachable_lines = value["projected_reachable_lines"]
        .as_u64()
        .expect("projected_reachable_lines");
    let projected_unreachable_lines = value["projected_unreachable_lines"]
        .as_u64()
        .expect("projected_unreachable_lines");
    assert!(total_lines >= code_lines, "json={stdout}");
    assert_eq!(
        projected_reachable_lines + projected_unreachable_lines,
        code_lines,
        "json={stdout}",
    );
    assert!(
        span_array_contains_function(&value["reachable_spans"], "live"),
        "json={stdout}",
    );
    assert!(
        span_array_contains_function(&value["reachable_spans"], "helper"),
        "json={stdout}",
    );
    assert!(
        span_array_contains_function(&value["unreachable_spans"], "orphan"),
        "json={stdout}",
    );

    fs::remove_dir_all(root).unwrap();
}
