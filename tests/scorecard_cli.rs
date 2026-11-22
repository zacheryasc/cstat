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
    let root = std::env::temp_dir().join(format!("cstat-scorecard-{name}-{unique}"));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
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
        "command failed: status={:?}\nstderr={}\nstdout={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
}

fn score(project: &Path) -> Value {
    let output = run_cstat(project, &["--json", "scorecard"]);
    assert_success(&output);
    serde_json::from_slice(&output.stdout).expect("parse scorecard JSON")
}

fn component(value: &Value, name: &str) -> f64 {
    value["component_costs"][name]["cost"]
        .as_f64()
        .unwrap_or_else(|| panic!("missing component {name}: {value}"))
}

fn total(value: &Value) -> f64 {
    value["code_complexity_cost"]
        .as_f64()
        .unwrap_or_else(|| panic!("missing total: {value}"))
}

fn write_main(root: &Path, source: &str) {
    fs::write(root.join("src/main.rs"), source).unwrap();
}

fn git(project: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(project)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", project.join("unused-git-config"))
        .args([
            "-c",
            "user.name=cstat test",
            "-c",
            "user.email=cstat@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("invoke git");
    assert_success(&output);
}

fn changed_score(project: &Path, base: Option<&str>) -> Value {
    let mut args = vec!["--json", "scorecard", "--top", "100", "--changed"];
    args.extend(base);
    let output = run_cstat(project, &args);
    assert_success(&output);
    serde_json::from_slice(&output.stdout).expect("parse changed scorecard JSON")
}

fn scored_files(report: &Value) -> Vec<&str> {
    let mut files = report["top_contributors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "file")
        .map(|item| item["file"].as_str().unwrap())
        .collect::<Vec<_>>();
    files.sort_unstable();
    files
}

#[test]
fn changed_scorecard_scores_current_files_relative_to_the_selected_commit() {
    let root = temp_project("changed");
    git(&root, &["init", "--quiet", "--template="]);
    write_main(&root, "fn main() {}\n");
    fs::write(root.join(".gitignore"), "src/ignored.rs\n").unwrap();
    for file in ["staged", "unstaged", "deleted"] {
        fs::write(
            root.join(format!("src/{file}.rs")),
            format!("fn {file}() {{}}\n"),
        )
        .unwrap();
    }
    git(&root, &["add", "."]);
    git(&root, &["commit", "--quiet", "-m", "baseline"]);
    git(&root, &["tag", "review-base"]);

    fs::write(root.join("src/committed.rs"), "fn committed() {}\n").unwrap();
    git(&root, &["add", "src/committed.rs"]);
    git(&root, &["commit", "--quiet", "-m", "add a function"]);

    let clean = changed_score(&root, None);
    assert_eq!(clean["metadata"]["rust_files"], 0);
    assert_eq!(total(&clean), 0.0);

    fs::write(root.join("src/staged.rs"), "fn staged_update() {}\n").unwrap();
    git(&root, &["add", "src/staged.rs"]);
    fs::write(root.join("src/unstaged.rs"), "fn working_copy_update() {}\n").unwrap();
    fs::remove_file(root.join("src/deleted.rs")).unwrap();
    fs::write(root.join("src/untracked file.rs"), "fn untracked() {}\n").unwrap();
    fs::write(root.join("src/ignored.rs"), "fn ignored() {}\n").unwrap();
    fs::write(root.join("build.rs"), "fn main() {}\n").unwrap();
    fs::write(root.join("README.md"), "Not Rust\n").unwrap();

    let head = changed_score(&root, None);
    assert_eq!(head["metadata"]["changed_since"], "HEAD");
    assert_eq!(
        scored_files(&head),
        ["build.rs", "src/staged.rs", "src/unstaged.rs", "src/untracked file.rs"]
    );
    assert!(head["top_contributors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["function"] == "working_copy_update"));

    let historical = changed_score(&root, Some("review-base"));
    assert_eq!(historical["metadata"]["changed_since"], "review-base");
    assert_eq!(
        scored_files(&historical),
        [
            "build.rs",
            "src/committed.rs",
            "src/staged.rs",
            "src/unstaged.rs",
            "src/untracked file.rs",
        ]
    );
    let single = changed_score(&root.join("src/staged.rs"), None);
    assert_eq!(scored_files(&single), ["src/staged.rs"]);

    let invalid = run_cstat(&root, &["--json", "scorecard", "--changed", "not-a-ref"]);
    assert!(!invalid.status.success());
    let error: Value = serde_json::from_slice(&invalid.stdout).unwrap();
    assert!(error["error"].as_str().unwrap().contains("not-a-ref"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scorecard_json_contract_is_stable_and_purely_structural() {
    let root = temp_project("json-contract");
    fs::create_dir_all(root.join("tests")).unwrap();
    fs::create_dir_all(root.join("benches")).unwrap();
    write_main(
        &root,
        r#"fn main() {
    live();
}

fn live() -> usize {
    1
}

fn branchy(input: usize) -> usize {
    let mut total = 0;
    if input > 0 { total += 1; }
    if input > 1 { total += 1; }
    if input > 2 { total += 1; }
    if input > 3 { total += 1; }
    if input > 4 { total += 1; }
    total
}
"#,
    );
    fs::write(
        root.join("tests/smoke.rs"),
        "#[test]\nfn smoke() { assert_eq!(1 + 1, 2); }\n",
    )
    .unwrap();
    fs::write(root.join("benches/bench.rs"), "fn bench_helper() {}\n").unwrap();

    let value = score(&root);
    assert_eq!(value["cstat_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["score_version"], "code_complexity_cost_v2");
    for field in [
        "target",
        "code_complexity_cost",
        "code_complexity_cost_per_kloc",
        "component_costs",
        "scope_breakdown",
        "top_contributors",
        "metadata",
    ] {
        assert!(value.get(field).is_some(), "missing {field}: {value}");
    }
    assert!(
        value["component_costs"].get("verification_gap").is_none(),
        "scorecard must not mix harness verification into code complexity: {value}",
    );
    assert!(
        value["component_costs"].get("api_surface").is_some(),
        "scorecard should expose API surface component: {value}",
    );
    for scope in ["production", "tests", "benches"] {
        assert!(
            value["scope_breakdown"].get(scope).is_some(),
            "missing scope {scope}: {value}",
        );
    }
    assert!(
        value["top_contributors"]
            .as_array()
            .expect("top array")
            .len()
            <= 20,
        "default top contributor count should be bounded: {value}",
    );

    let repeated = score(&root);
    assert_eq!(value, repeated, "scorecard should be deterministic");

    let human = run_cstat(&root, &["scorecard", "--top", "3"]);
    assert_success(&human);
    let stdout = String::from_utf8_lossy(&human.stdout);
    for expected in [
        "Code complexity score",
        "score version: code_complexity_cost_v2",
        "lower is cleaner; harness guards behavior separately",
        "components:",
        "api surface:",
        "top contributors:",
    ] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn deliberately_messy_code_scores_worse_than_simple_code() {
    let clean = temp_project("clean");
    write_main(
        &clean,
        r#"fn main() {
    println!("{}", live());
}

fn live() -> usize {
    1
}
"#,
    );

    let messy = temp_project("messy");
    let mut source = String::from(
        r#"fn main() {
    println!("{}", tangled(12));
}

fn tangled(input: usize) -> usize {
    let mut total = 0;
    if input > 0 { total += 1; }
    if input > 1 { total += 1; }
    if input > 2 { total += 1; }
    if input > 3 { total += 1; }
    if input > 4 { total += 1; }
    if input > 5 { total += 1; }
    if input > 6 { total += 1; }
    if input > 7 {
        if input > 8 {
            if input > 9 {
                total += input;
            }
        }
    }
"#,
    );
    for index in 0..70 {
        source.push_str(&format!("    let value_{index} = {index};\n"));
    }
    source.push_str("    total\n}\n");
    for index in 0..30 {
        source.push_str(&format!("fn helper_{index}() -> usize {{ {index} }}\n"));
    }
    write_main(&messy, &source);

    let clean_score = score(&clean);
    let messy_score = score(&messy);
    assert!(
        total(&messy_score) > total(&clean_score),
        "messy code should cost more: clean={clean_score} messy={messy_score}",
    );
    assert!(
        component(&messy_score, "function_complexity")
            > component(&clean_score, "function_complexity"),
        "messy function shape should cost more: clean={clean_score} messy={messy_score}",
    );
    assert!(
        component(&messy_score, "file_concentration")
            > component(&clean_score, "file_concentration"),
        "messy file concentration should cost more: clean={clean_score} messy={messy_score}",
    );

    fs::remove_dir_all(clean).unwrap();
    fs::remove_dir_all(messy).unwrap();
}

#[test]
fn adding_branching_nesting_and_large_body_does_not_reduce_function_cost() {
    let root = temp_project("function-monotonic");
    write_main(
        &root,
        r#"fn main() {
    println!("{}", value(1));
}

fn value(input: usize) -> usize {
    input + 1
}
"#,
    );
    let before = score(&root);

    let mut source = String::from(
        r#"fn main() {
    println!("{}", value(1));
}

fn value(input: usize) -> usize {
    let mut total = input;
    if input > 0 { total += 1; }
    if input > 1 { total += 1; }
    if input > 2 { total += 1; }
    if input > 3 { total += 1; }
    if input > 4 { total += 1; }
    if input > 5 {
        if input > 6 {
            if input > 7 {
                total += input;
            }
        }
    }
"#,
    );
    for index in 0..80 {
        source.push_str(&format!("    total += {index};\n"));
    }
    source.push_str("    total\n}\n");
    write_main(&root, &source);
    let after = score(&root);

    assert!(
        total(&after) > total(&before),
        "larger, branchier body should not reduce total cost: before={before} after={after}",
    );
    assert!(
        component(&after, "function_complexity") > component(&before, "function_complexity"),
        "larger, branchier body should increase function cost: before={before} after={after}",
    );
    assert!(
        component(&after, "file_concentration") >= component(&before, "file_concentration"),
        "larger body should not reduce file concentration: before={before} after={after}",
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn symbol_piles_increase_abstraction_surface_and_file_concentration() {
    let root = temp_project("symbol-pile");
    write_main(
        &root,
        r#"fn main() {
    println!("ok");
}
"#,
    );
    let before = score(&root);

    let mut source = String::from(
        r#"fn main() {
    println!("ok");
}
"#,
    );
    for index in 0..18 {
        source.push_str(&format!("struct Data{index};\n"));
    }
    for index in 0..12 {
        source.push_str(&format!("enum Choice{index} {{ A, B }}\n"));
    }
    for index in 0..30 {
        source.push_str(&format!("fn helper_{index}() -> usize {{ {index} }}\n"));
    }
    write_main(&root, &source);
    let after = score(&root);

    assert!(
        component(&after, "abstraction_surface") > component(&before, "abstraction_surface"),
        "symbol pile should increase abstraction surface: before={before} after={after}",
    );
    assert!(
        component(&after, "file_concentration") > component(&before, "file_concentration"),
        "symbol pile should increase file concentration: before={before} after={after}",
    );
    assert!(
        total(&after) > total(&before),
        "symbol pile should increase total cost: before={before} after={after}",
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fan_out_and_bidirectional_dependencies_increase_module_coupling() {
    let root = temp_project("coupling");
    fs::write(root.join("src/lib.rs"), "pub mod root;\npub mod a;\n").unwrap();
    fs::write(root.join("src/root.rs"), "pub fn touch() {}\n").unwrap();
    fs::write(root.join("src/a.rs"), "pub fn a() {}\n").unwrap();
    let before = score(&root);

    fs::write(
        root.join("src/lib.rs"),
        "pub mod root;\npub mod a;\npub mod b;\npub mod c;\npub mod d;\npub mod e;\npub mod f;\n",
    )
    .unwrap();
    fs::write(
        root.join("src/root.rs"),
        r#"use crate::a::a;
use crate::b::b;
use crate::c::c;
use crate::d::d;
use crate::e::e;
use crate::f::f;

pub fn touch() {
    a(); b(); c(); d(); e(); f();
}
"#,
    )
    .unwrap();
    fs::write(
        root.join("src/a.rs"),
        "use crate::root::touch;\npub fn a() { touch(); }\n",
    )
    .unwrap();
    for module in ["b", "c", "d", "e", "f"] {
        fs::write(
            root.join(format!("src/{module}.rs")),
            format!("pub fn {module}() {{}}\n"),
        )
        .unwrap();
    }
    let after = score(&root);

    assert!(
        component(&after, "module_coupling") > component(&before, "module_coupling"),
        "fan-out and bidirectional pair should increase coupling: before={before} after={after}",
    );
    assert!(
        total(&after) > total(&before),
        "coupling fixture should increase total cost: before={before} after={after}",
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn widening_visibility_increases_function_complexity_cost() {
    let root = temp_project("api-surface");
    write_main(
        &root,
        r#"fn main() {
    println!("{}", exposed(1));
}

fn exposed(input: usize) -> usize {
    input + 1
}
"#,
    );
    let private_score = score(&root);

    write_main(
        &root,
        r#"fn main() {
    println!("{}", exposed(1));
}

pub(crate) fn exposed(input: usize) -> usize {
    input + 1
}
"#,
    );
    let crate_score = score(&root);

    write_main(
        &root,
        r#"fn main() {
    println!("{}", exposed(1));
}

pub fn exposed(input: usize) -> usize {
    input + 1
}
"#,
    );
    let public_score = score(&root);

    assert!(
        component(&crate_score, "function_complexity") > component(&private_score, "function_complexity"),
        "pub(crate) visibility should add function complexity cost: private={private_score} crate={crate_score}",
    );
    assert!(
        component(&public_score, "function_complexity") > component(&crate_score, "function_complexity"),
        "pub visibility should cost more than pub(crate): crate={crate_score} public={public_score}",
    );
    assert!(
        total(&public_score) > total(&private_score),
        "widening visibility should increase total score: private={private_score} public={public_score}",
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn include_shards_are_scored_as_file_concentration_debt() {
    let root = temp_project("include-shard");
    fs::write(
        root.join("src/main.rs"),
        r#"include!("runtime.inc.rs");

fn main() {
    runtime();
}
"#,
    )
    .unwrap();
    fs::write(
        root.join("src/runtime.inc.rs"),
        r#"fn runtime() {
    let mut total = 0;
    for value in 0..10 {
        if value % 2 == 0 {
            total += value;
        } else {
            total -= value;
        }
    }
    println!("{total}");
}
"#,
    )
    .unwrap();

    let value = score(&root);
    let file_cost = component(&value, "file_concentration");
    let include = value["top_contributors"]
        .as_array()
        .expect("top contributors")
        .iter()
        .find(|item| item["kind"] == "include_shard")
        .unwrap_or_else(|| panic!("missing include shard contributor: {value}"));

    assert!(
        file_cost >= 200.0,
        "include shard penalty should land in file concentration: {value}",
    );
    assert_eq!(include["file"], "src/main.rs");
    assert_eq!(include["reasons"]["include_literal"], "runtime.inc.rs");
    assert!(
        include["cost"].as_f64().unwrap() >= 200.0,
        "include shard should carry base opacity cost: {include}",
    );

    fs::remove_dir_all(root).unwrap();
}
