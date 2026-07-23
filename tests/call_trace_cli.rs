use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_project(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("cstat-call-trace-cli-{name}-{unique}"));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src/main.rs"),
        "fn main() { parse(); }\nfn parse() { leaf(); }\nfn leaf() {}\n",
    )
    .unwrap();
    root
}

fn run_cstat(project: &Path, extra_args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_cstat");
    let mut command = Command::new(bin);
    command.args(["--no-color", "--path"]);
    command.arg(project);
    command.args(extra_args);
    command.output().expect("invoke cstat binary")
}

fn assert_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "cstat failed: status={:?}\nstderr={}\nstdout={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
}

#[test]
fn call_trace_infers_cargo_run_entry() {
    let root = temp_project("default");
    let output = run_cstat(&root, &["call-trace"]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cstat call trace"), "stdout={stdout}");
    assert!(
        stdout.contains("static possible-call expansion; not a runtime execution trace"),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("static expansion summary:"),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("cargo command: cargo run --"),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("entry: src/main.rs::main"),
        "stdout={stdout}"
    );
    assert!(stdout.contains("main"), "stdout={stdout}");
    assert!(stdout.contains("parse"), "stdout={stdout}");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn call_trace_honors_explicit_entry() {
    let root = temp_project("explicit");
    let output = run_cstat(&root, &["call-trace", "--entry", "src/main.rs::parse"]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("entry: src/main.rs::parse"),
        "stdout={stdout}"
    );
    assert!(stdout.contains("\nparse\n"), "stdout={stdout}");
    assert!(
        !stdout.contains("cargo command: cargo run --"),
        "stdout={stdout}"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn call_trace_entry_dir_lists_parseable_files() {
    let root = temp_project("entry-dir");
    fs::write(root.join("src/broken.rs"), "fn broken(\n").unwrap();

    let output = run_cstat(&root, &["call-trace", "--entry", "src"]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("parseable files under src:"),
        "stdout={stdout}"
    );
    assert!(stdout.contains("src/main.rs"), "stdout={stdout}");
    assert!(!stdout.contains("src/broken.rs"), "stdout={stdout}");
    assert!(!stdout.contains("summary:"), "stdout={stdout}");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn call_trace_entry_file_lists_entrypoints() {
    let root = temp_project("entry-file");

    let output = run_cstat(&root, &["call-trace", "--entry", "src/main.rs"]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("entrypoints in src/main.rs:"),
        "stdout={stdout}"
    );
    assert!(stdout.contains("src/main.rs::main"), "stdout={stdout}");
    assert!(stdout.contains("src/main.rs::parse"), "stdout={stdout}");
    assert!(stdout.contains("src/main.rs::leaf"), "stdout={stdout}");
    assert!(!stdout.contains("summary:"), "stdout={stdout}");

    fs::remove_dir_all(root).unwrap();
}
