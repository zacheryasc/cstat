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
    let root = std::env::temp_dir().join(format!("cstat-cluster-{name}-{unique}"));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src/lib.rs"),
        r#"
struct Config;
struct Worker;

impl Config {
    fn new(_args: Vec<String>) -> Self { Self }
}

impl Worker {
    fn new(_config: &Config) -> Self { Self }
    fn wait(&self, _config: &Config) -> bool { true }
}

fn serve(_worker: Worker, ready: bool) -> usize {
    if ready { 1 } else { 0 }
}

fn cleanup(value: usize) -> usize { value }

fn run(args: Vec<String>) -> usize {
    let config = Config::new(args);
    let worker = Worker::new(&config);
    let ready = worker.wait(&config);
    let result = serve(worker, ready);
    let mut total = 0usize;
    total = cleanup(result);
    total += 1;
    total
}
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
fn cluster_human_verbose_explains_value_clusters() {
    let root = temp_project("human");
    let file = root.join("src/lib.rs");
    let output = run_cstat(&file, &["cluster", "-v"]);
    assert_success(&output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Value clusters"), "stdout={stdout}");
    assert!(
        stdout.contains(
            "value = a function parameter or local let-binding collected from the Rust AST."
        ),
        "missing value explanation: {stdout}"
    );
    assert!(stdout.contains("references"), "stdout={stdout}");
    assert!(stdout.contains("mutations"), "stdout={stdout}");
    assert!(stdout.contains("connected values"), "stdout={stdout}");
    assert!(stdout.contains("fn run:"), "stdout={stdout}");
    assert!(stdout.contains("config"), "stdout={stdout}");
    assert!(stdout.contains("worker"), "stdout={stdout}");
    assert!(stdout.contains("total"), "stdout={stdout}");
    assert!(
        !stdout.contains("def      live uses  nbrs pressure"),
        "old header still present: {stdout}"
    );
    assert!(
        !stdout.contains("pressure"),
        "pressure still present: {stdout}"
    );
    assert!(
        !stdout.contains("Top signal"),
        "old signal text still present: {stdout}"
    );
    assert!(
        !stdout.contains("Use lines"),
        "old use-line text still present: {stdout}"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cluster_json_reports_def_use_clusters() {
    let root = temp_project("json");
    let file = root.join("src/lib.rs");
    let output = run_cstat(&file, &["--json", "cluster"]);
    assert_success(&output);

    let json: Value = serde_json::from_slice(&output.stdout).expect("cluster json");
    let functions = json
        .get("functions")
        .and_then(Value::as_array)
        .expect("functions array");
    let run = functions
        .iter()
        .find(|function| function.get("name").and_then(Value::as_str) == Some("run"))
        .expect("run function");
    let clusters = run
        .get("clusters")
        .and_then(Value::as_array)
        .expect("clusters array");
    let names = clusters
        .iter()
        .filter_map(|cluster| cluster.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(names.contains(&"config"), "names={names:?}");
    assert!(names.contains(&"worker"), "names={names:?}");
    let total = clusters
        .iter()
        .find(|cluster| cluster.get("name").and_then(Value::as_str) == Some("total"))
        .expect("total cluster");
    let mutation_lines = total
        .get("mutation_lines")
        .and_then(Value::as_array)
        .expect("mutation_lines array");
    assert_eq!(mutation_lines.len(), 2, "total={total}");
    for removed_key in [
        "pressure",
        "defined_line",
        "live_start",
        "live_end",
        "live_span",
    ] {
        assert!(
            total.get(removed_key).is_none(),
            "{removed_key} should not be present in total={total}"
        );
    }

    fs::remove_dir_all(root).unwrap();
}
