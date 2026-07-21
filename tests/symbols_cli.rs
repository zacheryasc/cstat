use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const LIB_RS: &str = r#"const LIMIT: usize = 10;
static NAME: &str = "fixture";

struct Parser;

enum Mode {
    Fast,
}

trait Parse {
    fn parse(&self);
}

impl Parser {
    fn new() -> Self {
        Self
    }
}

impl Parse for Parser {
    fn parse(&self) {}
}

fn free() {
    let _ = LIMIT;
}

mod inline {
    pub struct Inner;

    pub const FLAG: bool = true;

    pub fn nested() {}
}
"#;

fn temp_project(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("cstat-symbols-{name}-{unique}"));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), LIB_RS).unwrap();
    fs::write(root.join("src/broken.rs"), "fn broken(\n").unwrap();
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

fn parse_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("parse symbols JSON")
}

fn find_file<'a>(value: &'a Value, path: &str) -> &'a Value {
    value["files"]
        .as_array()
        .expect("files array")
        .iter()
        .find(|entry| entry["path"] == path)
        .unwrap_or_else(|| panic!("missing file row {path}: {value}"))
}

fn assert_zero_symbol_counts(value: &Value) {
    for field in [
        "total",
        "functions",
        "structs",
        "enums",
        "traits",
        "trait_impls",
        "consts",
        "statics",
    ] {
        assert_eq!(value[field], 0, "expected zero {field}: {value}");
    }
}

fn find_symbol<'a>(rows: &'a [Value], kind: &str, symbol: &str) -> &'a Value {
    rows.iter()
        .find(|row| row["kind"] == kind && row["symbol"] == symbol)
        .unwrap_or_else(|| panic!("missing {kind} symbol {symbol}: {rows:?}"))
}

#[test]
fn project_json_reports_trait_impl_contract_and_parse_errors() {
    let root = temp_project("project-json");
    let output = run_cstat(&root, &["--json", "symbols"]);
    assert_success(&output);

    let value = parse_json(&output);
    assert!(value.get("totals").is_some(), "json={value}");
    assert!(value.get("files").is_some(), "json={value}");

    let totals = &value["totals"];
    assert_eq!(totals["total"], 12);
    assert_eq!(totals["functions"], 4);
    assert_eq!(totals["structs"], 2);
    assert_eq!(totals["enums"], 1);
    assert_eq!(totals["traits"], 1);
    assert_eq!(totals["trait_impls"], 1);
    assert_eq!(totals["consts"], 2);
    assert_eq!(totals["statics"], 1);
    assert_eq!(totals["parse_error_files"], 1);
    assert!(
        totals.get("impls").is_none(),
        "old impls key present: {totals}"
    );

    let lib = find_file(&value, "src/lib.rs");
    assert_eq!(lib["total"], 12);
    assert_eq!(lib["functions"], 4);
    assert_eq!(lib["structs"], 2);
    assert_eq!(lib["enums"], 1);
    assert_eq!(lib["traits"], 1);
    assert_eq!(lib["trait_impls"], 1);
    assert_eq!(lib["consts"], 2);
    assert_eq!(lib["statics"], 1);
    assert_eq!(lib["parse_error"], false);
    assert!(lib.get("impls").is_none(), "old impls key present: {lib}");

    let broken = find_file(&value, "src/broken.rs");
    assert_eq!(broken["parse_error"], true);
    assert_zero_symbol_counts(broken);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_file_json_reports_rows_spans_and_schema() {
    let root = temp_project("selected-json");
    let file = root.join("src/lib.rs");
    let output = run_cstat(&file, &["--json", "symbols"]);
    assert_success(&output);

    let value = parse_json(&output);
    for field in [
        "file",
        "total",
        "functions",
        "structs",
        "enums",
        "traits",
        "trait_impls",
        "consts",
        "statics",
        "parse_error",
        "symbols",
    ] {
        assert!(value.get(field).is_some(), "missing {field}: {value}");
    }
    assert_eq!(value["file"], "src/lib.rs");
    assert_eq!(value["parse_error"], false);
    assert_eq!(value["total"], 12);
    assert_eq!(value["functions"], 4);
    assert_eq!(value["structs"], 2);
    assert_eq!(value["enums"], 1);
    assert_eq!(value["traits"], 1);
    assert_eq!(value["trait_impls"], 1);
    assert_eq!(value["consts"], 2);
    assert_eq!(value["statics"], 1);
    assert!(
        value.get("impls").is_none(),
        "old impls key present: {value}"
    );

    let rows = value["symbols"].as_array().expect("symbols array");
    let fn_rows = rows.iter().filter(|row| row["kind"] == "fn").count();
    assert_eq!(fn_rows, 4, "rows={rows:?}");

    let free = find_symbol(rows, "fn", "free");
    assert_eq!(free["line_start"], 24);
    assert_eq!(free["line_end"], 26);

    let inherent_method = find_symbol(rows, "fn", "Parser::new");
    assert_eq!(inherent_method["line_start"], 15);
    assert_eq!(inherent_method["line_end"], 17);

    find_symbol(rows, "fn", "inline::nested");

    let trait_impl_rows: Vec<&Value> = rows
        .iter()
        .filter(|row| row["kind"] == "trait_impl")
        .collect();
    assert_eq!(trait_impl_rows.len(), 1, "rows={rows:?}");
    let trait_impl = trait_impl_rows[0];
    assert_eq!(trait_impl["symbol"], "<Parser as Parse>");
    assert_eq!(trait_impl["line_start"], 20);
    assert_eq!(trait_impl["line_end"], 22);

    find_symbol(rows, "const", "inline::FLAG");
    find_symbol(rows, "static", "NAME");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_file_json_surfaces_parse_error() {
    let root = temp_project("parse-error");
    let file = root.join("src/broken.rs");
    let output = run_cstat(&file, &["--json", "symbols"]);
    assert_success(&output);

    let value = parse_json(&output);
    assert_eq!(value["file"], "src/broken.rs");
    assert_eq!(value["parse_error"], true);
    assert_zero_symbol_counts(&value);
    assert!(
        value["symbols"]
            .as_array()
            .expect("symbols array")
            .is_empty(),
        "json={value}"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn symbols_human_verbose_explains_contract() {
    let root = temp_project("human-verbose");
    let output = run_cstat(&root, &["-v", "symbols"]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "Rust AST",
        "syn",
        "semantic name resolution",
        "public API usage",
        "trait_impl",
        "inherent",
        "Parse-error",
        "const",
        "static",
    ] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn symbols_help_mentions_file_path_and_trait_impls() {
    let root = temp_project("help");

    let root_help = run_cstat(&root, &["--help"]);
    assert_success(&root_help);
    let root_stdout = String::from_utf8_lossy(&root_help.stdout);
    assert!(
        root_stdout.contains("Rust project directory or Rust source file"),
        "stdout={root_stdout}"
    );

    let symbols_help = run_cstat(&root, &["symbols", "--help"]);
    assert_success(&symbols_help);
    let symbols_stdout = String::from_utf8_lossy(&symbols_help.stdout);
    assert!(
        symbols_stdout.contains("Rust AST symbol counts"),
        "stdout={symbols_stdout}"
    );
    assert!(
        symbols_stdout.contains("trait impl"),
        "stdout={symbols_stdout}"
    );

    fs::remove_dir_all(root).unwrap();
}
