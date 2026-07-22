use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const FIXTURE: &str = r#"pub trait Worker {
    fn work(&self) -> usize;
}

pub struct Engine<T> {
    value: T,
}

impl<T> Worker for Engine<T>
where
    T: Copy + Into<usize>,
{
    fn work(&self) -> usize {
        self.value.into()
    }
}

pub fn simple() -> usize {
    1
}

pub fn branchy(input: i32) -> i32 {
    let mut total = 0;
    if input > 0 {
        total += 1;
    }
    match input {
        0 => total += 10,
        1 | 2 => total += 20,
        _ => total += 30,
    }
    for i in 0..input {
        if i % 2 == 0 {
            total += i;
        }
    }
    total
}

pub fn hard_signature<T, U>(
    first: T,
    second: U,
    count: usize,
) -> Result<Option<(T, U)>, String>
where
    T: Clone + Into<String>,
    U: Default,
{
    if count == 0 {
        return Err(String::new());
    }
    Ok(Some((first, second)))
}
"#;

fn temp_project(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("cstat-function-probes-{name}-{unique}"));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), FIXTURE).unwrap();
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
        "cstat failed: status={:?} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_json(output: Output) -> Value {
    assert_success(&output);
    serde_json::from_slice(&output.stdout).expect("parse cstat json")
}

fn function_row<'a>(value: &'a Value, function: &str) -> &'a Value {
    value["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|row| row["function"] == function)
        .unwrap_or_else(|| panic!("missing function row {function}: {value}"))
}

fn assert_absent(row: &Value, keys: &[&str]) {
    for key in keys {
        assert!(row.get(*key).is_none(), "unexpected key {key}: {row}");
    }
}

#[test]
fn branching_json_reports_decision_path_fields() {
    let root = temp_project("branching-json");
    let value = parse_json(run_cstat(&root, &["--json", "branching"]));
    let row = function_row(&value, "branchy");

    assert_eq!(row["file"], "src/lib.rs");
    assert_eq!(row["branch_points"], 5);
    assert_eq!(row["cyclomatic"], 6);
    assert_eq!(row["nesting_depth"], 2);
    assert_absent(
        row,
        &[
            "cognitive",
            "generic_complexity",
            "param_complexity",
            "composite_score",
        ],
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn signature_json_reports_api_boundary_fields() {
    let root = temp_project("signature-json");
    let value = parse_json(run_cstat(&root, &["--json", "signature"]));
    let row = function_row(&value, "hard_signature");

    assert_eq!(row["file"], "src/lib.rs");
    assert_eq!(row["param_count"], 3);
    assert_eq!(row["return_type_complexity"], 6);
    assert_eq!(row["generic_param_count"], 2);
    assert_eq!(row["trait_bound_count"], 3);
    assert_eq!(row["where_predicate_count"], 2);
    assert_eq!(row["signature_score"], 16);
    assert_absent(
        row,
        &["cyclomatic", "cognitive", "line_count", "composite_score"],
    );

    let file_row = value["files"]
        .as_array()
        .expect("files array")
        .iter()
        .find(|row| row["file"] == "src/lib.rs")
        .unwrap_or_else(|| panic!("missing file row: {value}"));
    assert_eq!(file_row["trait_impl_count"], 1);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn span_json_reports_body_size_and_line_ranges() {
    let root = temp_project("span-json");
    let value = parse_json(run_cstat(&root, &["--json", "span"]));
    let row = function_row(&value, "branchy");

    assert_eq!(row["file"], "src/lib.rs");
    assert_eq!(row["line_start"], 22);
    assert_eq!(row["line_end"], 38);
    assert_eq!(row["line_count"], 17);
    assert_eq!(row["body_stmt_count"], 5);
    assert_absent(
        row,
        &[
            "cyclomatic",
            "cognitive",
            "signature_score",
            "composite_score",
        ],
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn probe_human_outputs_use_focused_headings() {
    let root = temp_project("human");

    let branching = run_cstat(&root, &["branching"]);
    assert_success(&branching);
    let branching_stdout = String::from_utf8_lossy(&branching.stdout);
    assert!(
        branching_stdout.contains("Function Branching Complexity"),
        "stdout={branching_stdout}"
    );
    assert!(
        !branching_stdout.contains("Score"),
        "stdout={branching_stdout}"
    );

    let signature = run_cstat(&root, &["signature"]);
    assert_success(&signature);
    let signature_stdout = String::from_utf8_lossy(&signature.stdout);
    assert!(
        signature_stdout.contains("Function Signature Complexity"),
        "stdout={signature_stdout}"
    );
    assert!(
        !signature_stdout.contains("Cycl"),
        "stdout={signature_stdout}"
    );

    let span = run_cstat(&root, &["span"]);
    assert_success(&span);
    let span_stdout = String::from_utf8_lossy(&span.stdout);
    assert!(
        span_stdout.contains("Function Implementation Span"),
        "stdout={span_stdout}"
    );
    assert!(!span_stdout.contains("Signature"), "stdout={span_stdout}");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_file_paths_scope_probe_json() {
    let root = temp_project("selected-file");
    let file = root.join("src/lib.rs");

    for command in ["branching", "signature", "span"] {
        let value = parse_json(run_cstat(&file, &["--json", command]));
        let functions = value["functions"].as_array().expect("functions array");
        assert!(!functions.is_empty(), "no functions for {command}: {value}");
        for row in functions {
            assert_eq!(row["file"], "src/lib.rs", "row={row}");
        }
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn old_complexity_command_is_not_available() {
    let root = temp_project("old-complexity");
    let output = run_cstat(&root, &["complexity"]);
    assert!(
        !output.status.success(),
        "old complexity command unexpectedly succeeded: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn help_lists_function_probe_commands() {
    let root = temp_project("help");
    let output = run_cstat(&root, &["--help"]);
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);

    for command in ["branching", "signature", "span"] {
        assert!(
            stdout.contains(command),
            "root help missing {command}: {stdout}"
        );
    }
    assert!(
        !stdout.lines().any(|line| line.starts_with("  complexity")),
        "root help still listed complexity: {stdout}"
    );

    fs::remove_dir_all(root).unwrap();
}
