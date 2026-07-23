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
    let root = std::env::temp_dir().join(format!("cstat-deps-cli-{name}-{unique}"));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "pub mod alpha;\npub mod beta;\n").unwrap();
    fs::write(
        root.join("src/alpha.rs"),
        r#"pub struct Shared;

pub fn shared(input: Shared) -> Shared {
    input
}

pub fn closure_owner(input: Shared) -> Shared {
    let first = || shared(input);
    let second = || shared(Shared);
    first();
    second()
}

pub fn sibling(input: Shared) -> Shared {
    shared(input)
}
"#,
    )
    .unwrap();
    fs::write(
        root.join("src/beta.rs"),
        r#"use crate::alpha::shared;

pub fn beta() {
    let _ = shared(crate::alpha::Shared);
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
fn deps_human_verbose_shows_actual_one_way_direction() {
    let root = temp_project("human-direction");
    let output = run_cstat(&root, &["deps", "--top", "10", "-v"]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Cells are pair strength, not edge direction"),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("One-way rows show actual source module -> target module direction."),
        "stdout={stdout}"
    );
    assert!(stdout.contains("signature type tokens"), "stdout={stdout}");
    let directed_line = stdout
        .lines()
        .find(|line| line.contains("beta") && line.contains("alpha") && line.contains("one-way"))
        .unwrap_or_else(|| panic!("missing beta -> alpha coupling line: {stdout}"));
    assert!(
        directed_line.find("beta") < directed_line.find("alpha"),
        "line should show source before target: {directed_line}"
    );
    assert!(
        !stdout.contains("alpha::closure_owner"),
        "function-scope pseudo-module leaked into cohesion output: {stdout}"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn deps_json_coupling_edges_and_cohesion_use_source_modules() {
    let root = temp_project("json-direction");
    let output = run_cstat(&root, &["--json", "deps"]);
    assert_success(&output);

    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: Value = serde_json::from_str(&stdout).expect("parse deps JSON");
    let pairs = value["coupling_pairs"]
        .as_array()
        .expect("coupling_pairs array");
    let alpha_beta = pairs
        .iter()
        .find(|pair| pair["module_a"] == "alpha" && pair["module_b"] == "beta")
        .unwrap_or_else(|| panic!("missing alpha/beta pair: {value}"));
    assert_eq!(alpha_beta["direction"], "unidirectional");
    assert_eq!(
        alpha_beta["edges"],
        serde_json::json!([{ "from": "beta", "to": "alpha" }]),
        "pair={alpha_beta}"
    );

    let cohesion = value["cohesion"].as_array().expect("cohesion array");
    let modules = cohesion
        .iter()
        .filter_map(|row| row["module"].as_str())
        .collect::<Vec<_>>();
    assert!(modules.contains(&"alpha"), "modules={modules:?}");
    assert!(modules.contains(&"beta"), "modules={modules:?}");
    assert!(
        !modules
            .iter()
            .any(|module| module.contains("closure_owner")),
        "cohesion should use source modules, not function scopes: {modules:?}"
    );
    let alpha = cohesion
        .iter()
        .find(|row| row["module"] == "alpha")
        .unwrap_or_else(|| panic!("missing alpha cohesion row: {value}"));
    assert_eq!(alpha["function_count"], 3);

    fs::remove_dir_all(root).unwrap();
}

fn nested_project(name: &str) -> PathBuf {
    let root = temp_project(name);
    fs::remove_file(root.join("src/alpha.rs")).unwrap();
    fs::remove_file(root.join("src/beta.rs")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub mod parent;\n").unwrap();
    fs::create_dir_all(root.join("src/parent")).unwrap();
    fs::write(
        root.join("src/parent/mod.rs"),
        r#"mod child;

pub struct Shared;

pub use child::child;
"#,
    )
    .unwrap();
    fs::write(
        root.join("src/parent/child.rs"),
        r#"use super::Shared;

pub fn child(_: Shared) {}
"#,
    )
    .unwrap();
    root
}

#[test]
fn deps_json_resolves_nested_mod_and_super_as_bidirectional_pair() {
    let root = nested_project("nested-bidir");
    let output = run_cstat(&root, &["--json", "deps"]);
    assert_success(&output);

    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: Value = serde_json::from_str(&stdout).expect("parse deps JSON");
    let pairs = value["coupling_pairs"]
        .as_array()
        .expect("coupling_pairs array");
    let parent_child = pairs
        .iter()
        .find(|pair| pair["module_a"] == "parent" && pair["module_b"] == "parent::child")
        .unwrap_or_else(|| panic!("missing parent/child pair: {value}"));
    assert_eq!(parent_child["direction"], "bidirectional");
    assert_eq!(
        parent_child["edges"],
        serde_json::json!([
            { "from": "parent", "to": "parent::child" },
            { "from": "parent::child", "to": "parent" }
        ]),
        "pair={parent_child}"
    );

    fs::remove_dir_all(root).unwrap();
}
