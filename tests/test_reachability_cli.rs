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

fn json_array_contains_suffix(array: &Value, suffix: &str) -> bool {
    array
        .as_array()
        .expect("json array")
        .iter()
        .filter_map(Value::as_str)
        .any(|value| value.ends_with(suffix))
}

fn json_edges_contain_suffix(array: &Value, caller_suffix: &str, callee_suffix: &str) -> bool {
    array
        .as_array()
        .expect("json edge array")
        .iter()
        .any(|edge| {
            edge["caller"]
                .as_str()
                .is_some_and(|caller| caller.ends_with(caller_suffix))
                && edge["callee"]
                    .as_str()
                    .is_some_and(|callee| callee.ends_with(callee_suffix))
        })
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
fn stale_summary_report_and_advanced_commands_are_unavailable() {
    let root = temp_project("stale-commands");

    for args in [
        &["summary"][..],
        &["report"][..],
        &["advanced"][..],
        &["advanced", "flow-heatmap", "--walks", "10"][..],
    ] {
        let output = run_cstat(&root, args);
        assert!(
            !output.status.success(),
            "stale command unexpectedly succeeded for {args:?}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn help_lists_focused_commands_and_omits_pruned_commands() {
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
    for pruned in ["summary", "report", "advanced", "datapaths"] {
        assert!(
            !root_stdout
                .lines()
                .any(|line| line.trim_start().starts_with(pruned)),
            "help still listed pruned command {pruned}: {root_stdout}"
        );
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_file_reachability_covers_mvp_chat_false_negative_patterns() {
    let root = temp_project("mvp-chat-patterns");
    let bin_dir = root.join("src/bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let file = bin_dir.join("mvp_chat.rs");
    fs::write(
        &file,
        r#"
struct RuntimeConfigProfile;
struct RelayMode;
struct Config {
    profile: RuntimeConfigProfile,
    relay_mode: RelayMode,
    model_path: Option<&'static str>,
}

impl RuntimeConfigProfile {
    fn as_str(&self) -> &'static str { "docker" }
    fn default_provider(&self) -> &'static str { "vastai" }
}

impl RelayMode {
    fn as_str(&self) -> &'static str { "auto" }
}

fn relay_mode_env_value(_mode: &str) -> &'static str { "1" }
fn default_cached_model_path() -> &'static str { "/tmp/model" }
fn run_status() -> bool { true }
fn request_stop() {}

impl Config {
    fn orchestrator_cli_args(&self) {
        let mut args = Vec::new();
        args.extend([
            "--relay-mode".to_owned(),
            relay_mode_env_value(self.relay_mode.as_str()).to_owned(),
        ]);
        self.profile.as_str();
        self.model_path.unwrap_or_else(default_cached_model_path);
    }
}

struct ParsedArgs { provider: Option<&'static str> }
impl ParsedArgs {
    fn apply_profile(&mut self, config_profile: RuntimeConfigProfile) {
        self.set_provider_selector(config_profile.default_provider());
    }
    fn set_provider_selector(&mut self, provider: &'static str) {
        self.provider = Some(provider);
    }
}

trait VastAiApproval {
    fn ask(&mut self) -> bool;
}

struct StdinVastAiApproval;

impl VastAiApproval for StdinVastAiApproval {
    fn ask(&mut self) -> bool { true }
}

fn confirm_vastai_approval<P: VastAiApproval>(approval: &mut P) -> bool {
    approval.ask()
}

struct OfferPreviewer;
struct Offer {
    previewer: OfferPreviewer,
}

impl OfferPreviewer {
    fn preview(&self) {}
}

impl Offer {
    fn show(&self) {
        self.previewer.preview();
    }
}

struct Command;
struct OrchChild(Command);

impl OrchChild {
    fn shutdown(&mut self, _wait: bool) {}
}

impl Drop for OrchChild {
    fn drop(&mut self) {
        self.shutdown(false);
    }
}

fn consume_child(_child: OrchChild) {}

fn ensure_orch_binary() -> bool {
    ensure_orch_binary_with("config", "root", "default-orch", run_status)
}

fn ensure_orch_binary_with(
    _config: &str,
    _root: &str,
    _default_orch: &str,
    run_status_fn: fn() -> bool,
) -> bool {
    run_status_fn()
}

fn install_signal_handler() {
    unsafe {
        libc::signal(15, request_stop as *const () as usize);
    }
}

struct MutRunner;
impl MutRunner {
    fn run(&mut self, value: &'static str) {
        nested_method_leaf(value);
    }
}
fn nested_arg() -> &'static str {
    nested_free_leaf();
    "x"
}
fn nested_free_leaf() {}
fn nested_method_leaf(_value: &'static str) {}

fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_selected_file_patterns() {
        let config = Config {
            profile: RuntimeConfigProfile,
            relay_mode: RelayMode,
            model_path: None,
        };
        config.orchestrator_cli_args();
        let mut parsed = ParsedArgs { provider: None };
        parsed.apply_profile(RuntimeConfigProfile);
        ensure_orch_binary();
        let mut runner = MutRunner;
        runner.run(nested_arg());
        let mut approval = StdinVastAiApproval;
        confirm_vastai_approval(&mut approval);
        let offer = Offer { previewer: OfferPreviewer };
        offer.show();
        let child = OrchChild(Command);
        consume_child(child);
        install_signal_handler();
    }
}
"#,
    )
    .unwrap();

    let output = run_cstat(&file, &["--json", "test-reachability"]);
    assert_success(&output);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: Value = serde_json::from_str(&stdout).expect("parse selected reachability JSON");
    assert_eq!(value["file"], "src/bin/mvp_chat.rs");
    assert!(
        json_array_contains_suffix(&value["wrapper_entrypoints"], "main"),
        "wrapper_entrypoints={:?}",
        value["wrapper_entrypoints"]
    );
    assert!(
        !json_array_contains_suffix(&value["unreachable_functions"], "main"),
        "main should be classified as wrapper, not uncovered production: {stdout}"
    );

    let expected_suffixes = [
        "ParsedArgs::apply_profile",
        "ParsedArgs::set_provider_selector",
        "RuntimeConfigProfile::as_str",
        "RuntimeConfigProfile::default_provider",
        "relay_mode_env_value",
        "default_cached_model_path",
        "ensure_orch_binary",
        "ensure_orch_binary_with",
        "run_status",
        "request_stop",
        "StdinVastAiApproval::ask",
        "OfferPreviewer::preview",
        "MutRunner::run",
        "nested_arg",
        "nested_free_leaf",
        "nested_method_leaf",
        "OrchChild::drop",
        "OrchChild::shutdown",
    ];

    for suffix in expected_suffixes {
        assert!(
            json_array_contains_suffix(&value["reachable_functions"], suffix),
            "reachable_functions missing {suffix}: {stdout}"
        );
        assert!(
            !json_array_contains_suffix(&value["unreachable_functions"], suffix),
            "unreachable_functions still contains {suffix}: {stdout}"
        );
    }
    assert_eq!(
        value["unreachable_function_count"], 0,
        "selected-file production functions should all be reachable: {stdout}"
    );

    for (caller, callee) in [
        (
            "Config::orchestrator_cli_args",
            "RuntimeConfigProfile::as_str",
        ),
        ("Config::orchestrator_cli_args", "RelayMode::as_str"),
        ("Config::orchestrator_cli_args", "relay_mode_env_value"),
        ("Config::orchestrator_cli_args", "default_cached_model_path"),
        (
            "ParsedArgs::apply_profile",
            "ParsedArgs::set_provider_selector",
        ),
        (
            "ParsedArgs::apply_profile",
            "RuntimeConfigProfile::default_provider",
        ),
        ("ensure_orch_binary", "ensure_orch_binary_with"),
        ("ensure_orch_binary_with", "run_status"),
        ("install_signal_handler", "request_stop"),
        ("confirm_vastai_approval", "StdinVastAiApproval::ask"),
        ("Offer::show", "OfferPreviewer::preview"),
        ("MutRunner::run", "nested_method_leaf"),
        ("tests::covers_selected_file_patterns", "nested_arg"),
        ("nested_arg", "nested_free_leaf"),
        ("OrchChild::drop", "OrchChild::shutdown"),
    ] {
        assert!(
            json_edges_contain_suffix(&value["reachable_edges"], caller, callee),
            "reachable_edges missing {caller} -> {callee}: {stdout}"
        );
    }

    let dead_output = run_cstat(&file, &["--json", "dead-code"]);
    assert_success(&dead_output);
    let dead_stdout = String::from_utf8(dead_output.stdout).unwrap();
    let dead_value: Value = serde_json::from_str(&dead_stdout).expect("parse dead-code JSON");
    let dead_functions = dead_value["functions"]
        .as_array()
        .expect("dead-code functions");
    for suffix in expected_suffixes {
        let row = dead_functions
            .iter()
            .find(|entry| {
                entry["function"]
                    .as_str()
                    .is_some_and(|function| function.ends_with(suffix))
            })
            .unwrap_or_else(|| panic!("dead-code row missing {suffix}: {dead_stdout}"));
        assert_eq!(
            row["candidate"], false,
            "dead-code still marks {suffix} as candidate: {dead_stdout}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}
