use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct Project(PathBuf);

impl Project {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!(
            "cstat-cpp-{name}-{}-{nonce}",
            std::process::id(),
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn write(&self, path: &str, source: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }

    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command
            .current_dir(&self.0)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.0.join("unused-git-config"));
        command
    }

    fn run(&self, path: &str, args: &[&str]) -> Output {
        self
            .command(env!("CARGO_BIN_EXE_cstat"))
            .args(["--no-color", "--json", "--language", "cpp", "--path"])
            .arg(self.0.join(path))
            .args(args)
            .output()
            .expect("invoke cstat")
    }

    fn score(&self, path: &str, args: &[&str]) -> Value {
        let mut command = vec!["scorecard"];
        command.extend_from_slice(args);
        let output = self.run(path, &command);
        assert_success(&output);
        serde_json::from_slice(&output.stdout).expect("parse C++ scorecard JSON")
    }

    fn git(&self, args: &[&str]) {
        let output = self
            .command("git")
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
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed ({}):\nstderr={}\nstdout={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
}

fn assert_selection(report: &Value, files: &[&str]) {
    assert_eq!(report["status"], "complete", "{report}");
    assert_eq!(report["selected_files"], json!(files));
}

#[cfg(unix)]
struct RestrictedPath {
    path: PathBuf,
    original: fs::Permissions,
}

#[cfg(unix)]
impl RestrictedPath {
    fn new(path: PathBuf) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let original = fs::metadata(&path).unwrap().permissions();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        Self { path, original }
    }
}

#[cfg(unix)]
impl Drop for RestrictedPath {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, self.original.clone());
    }
}

#[cfg(unix)]
fn read_is_denied(path: &Path) -> bool {
    match fs::File::open(path) {
        // Root and callers with DAC bypass may still read a mode-000 path.
        Ok(_) => false,
        Err(error) => {
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
            true
        }
    }
}

#[cfg(unix)]
fn assert_incomplete(output: Output) -> Value {
    assert!(!output.status.success(), "{output:?}");
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "incomplete", "{report}");
    assert!(report.get("code_complexity_cost").is_none(), "{report}");
    report
}

#[test]
fn includes_use_ordered_search_tiers_and_keep_non_edges_distinct() {
    let p = Project::new("include-search");
    p.write(
        "project/src/main.cpp",
        concat!(
            "#include \"choice.h\"\n",
            "#include \"choice.h\"\n",
            "#include <choice.h>\n",
            "#include <local_only.h>\n",
            "#include <clash.h>\n",
            "#include <outside.h>\n",
            "#include \"../skip/excluded.h\"\n",
            "#include HEADER_NAME\n",
        ),
    );
    p.write("project/src/peer.cc", "#include \"choice.h\"\n");
    p.write("project/src/choice.h", "int shared();\n");
    p.write("project/src/local_only.h", "// angle lookup must not use this\n");
    p.write("project/inc/choice.h", "// root lookup\n");
    p.write("project/inc/clash.h", "// first candidate\n");
    p.write("project/other/clash.h", "// distinct second candidate\n");
    p.write("project/skip/excluded.h", "// present but ineligible\n");
    p.write("outside/outside.h", "// outside analysis root\n");

    let report = p.score(
        "project",
        &[
            "--include-dir", "inc", "--include-dir", "other", "--include-dir", "../outside",
            "--exclude", "skip",
        ],
    );
    assert_selection(
        &report,
        &[
            "inc/choice.h", "inc/clash.h", "other/clash.h", "src/choice.h", "src/local_only.h",
            "src/main.cpp", "src/peer.cc",
        ],
    );
    let graph = &report["includes"];
    let directives = graph["directives"].as_array().unwrap();
    let expected = [
        ("choice.h", "internal", Some("src/choice.h"), 1),
        ("choice.h", "internal", Some("src/choice.h"), 1),
        ("choice.h", "internal", Some("inc/choice.h"), 1),
        ("local_only.h", "unresolved", None, 0),
        ("clash.h", "ambiguous", None, 2),
        ("outside.h", "external", None, 1),
        ("../skip/excluded.h", "external", None, 1),
        ("HEADER_NAME", "macro", None, 0),
        ("choice.h", "internal", Some("src/choice.h"), 1),
    ];
    assert_eq!(directives.len(), expected.len());
    for (index, (directive, (spelling, status, target, candidates))) in
        directives.iter().zip(expected).enumerate()
    {
        let (source, line) = if index < 8 {
            ("src/main.cpp", index + 1)
        } else {
            ("src/peer.cc", 1)
        };
        assert_eq!(directive["source"], source);
        assert_eq!(directive["span"]["line_start"], line);
        assert_eq!(directive["spelling"], spelling);
        assert_eq!(directive["status"], status, "{directive}");
        assert_eq!(directive["target"], json!(target));
        assert_eq!(directive["candidates"].as_array().unwrap().len(), candidates);
    }
    let ambiguous_paths = directives[4]["candidates"].as_array().unwrap();
    assert!(Path::new(ambiguous_paths[0].as_str().unwrap()).ends_with("inc/clash.h"));
    assert!(Path::new(ambiguous_paths[1].as_str().unwrap()).ends_with("other/clash.h"));
    for (index, suffix) in [(5, "outside/outside.h"), (6, "skip/excluded.h")] {
        // Excluded and out-of-root files both exist, but neither is an internal target.
        // A candidate can preserve the original include spelling, including `..`.
        let candidate = directives[index]["candidates"][0].as_str().unwrap();
        assert!(Path::new(candidate).ends_with(suffix), "{candidate}");
    }
    assert_eq!(
        graph["edges"],
        json!([
            {"source": "src/main.cpp", "target": "inc/choice.h", "cyclic": false},
            {"source": "src/main.cpp", "target": "src/choice.h", "cyclic": false},
            {"source": "src/peer.cc", "target": "src/choice.h", "cyclic": false},
        ]),
    );
    let shared = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["path"] == "src/choice.h")
        .unwrap();
    assert_eq!(shared["fan_in"], 2);
    assert_eq!(report["inputs"]["named_surface"], 1);
    assert_eq!(report["components"]["declaration_surface_cost"], 2);
    assert_eq!(report["components"]["include_coupling_cost"], 6);
}

#[test]
fn spliced_quoted_include_names_keep_raw_spellings_and_share_an_edge() {
    let p = Project::new("include-splices");
    p.write("main.cpp", "#include \"fi\\\nrst.hpp\"\n#include \"fir\\\r\nst.hpp\"\r\n");
    p.write("first.hpp", "int first();\n");

    let report = p.score(".", &[]);
    assert_selection(&report, &["first.hpp", "main.cpp"]);
    let directives = report["includes"]["directives"].as_array().unwrap();
    assert_eq!(directives.len(), 2);
    for (directive, spelling) in directives.iter().zip(["fi\\\nrst.hpp", "fir\\\r\nst.hpp"]) {
        assert_eq!(directive["spelling"], spelling);
        assert_eq!(directive["status"], "internal");
        assert_eq!(directive["target"], "first.hpp");
    }
    assert_eq!(
        report["includes"]["edges"],
        json!([{"source": "main.cpp", "target": "first.hpp", "cyclic": false}]),
    );
    assert_eq!(report["components"]["include_coupling_cost"], 2);
}

#[test]
fn cycles_depend_on_selected_sources_but_unselected_headers_remain_targets() {
    let p = Project::new("cycles");
    p.git(&["init", "--quiet", "--template="]);
    p.write("src/a.cpp", "#include \"../b.h\"\n");
    p.write("b.h", "#include \"src/a.cpp\"\nint other();\n");
    p.write("src/self.hpp", "#include \"self.hpp\"\n");

    let closed = p.score(".", &[]);
    assert_selection(&closed, &["b.h", "src/a.cpp", "src/self.hpp"]);
    assert_eq!(
        closed["includes"]["cyclic_components"],
        json!([["b.h", "src/a.cpp"], ["src/self.hpp"]]),
    );
    assert_eq!(
        closed["includes"]["edges"],
        json!([
            {"source": "b.h", "target": "src/a.cpp", "cyclic": true},
            {"source": "src/a.cpp", "target": "b.h", "cyclic": true},
            {"source": "src/self.hpp", "target": "src/self.hpp", "cyclic": true},
        ]),
    );
    assert_eq!(
        closed["includes"]["nodes"],
        json!([
            {"path": "b.h", "fan_in": 1, "fan_out": 1, "cyclic_edges": 1},
            {"path": "src/a.cpp", "fan_in": 1, "fan_out": 1, "cyclic_edges": 1},
            {"path": "src/self.hpp", "fan_in": 1, "fan_out": 1, "cyclic_edges": 1},
        ]),
    );
    assert_eq!(closed["inputs"]["cyclic_edges"], 3);
    assert_eq!(closed["components"]["include_coupling_cost"], 15);

    let selected = p.score("src/a.cpp", &[]);
    assert_selection(&selected, &["src/a.cpp"]);
    assert_eq!(selected["selection"]["mode"], "file");
    assert_eq!(selected["includes"]["directives"].as_array().unwrap().len(), 1);
    assert_eq!(
        selected["includes"]["edges"],
        json!([{"source": "src/a.cpp", "target": "b.h", "cyclic": false}]),
    );
    assert_eq!(selected["includes"]["cyclic_components"], json!([]));
    assert_eq!(
        selected["includes"]["nodes"],
        json!([
            {"path": "b.h", "fan_in": 1, "fan_out": 0, "cyclic_edges": 0},
            {"path": "src/a.cpp", "fan_in": 0, "fan_out": 1, "cyclic_edges": 0},
        ]),
    );
    assert_eq!(selected["inputs"]["named_surface"], 0);
    assert_eq!(selected["components"]["include_coupling_cost"], 2);
}

#[test]
fn mixed_tree_discovery_applies_only_cpp_extensions_git_ignores_and_excludes() {
    let p = Project::new("discovery");
    p.git(&["init", "--quiet", "--template="]);
    p.write(".gitignore", "/ignored/\n");
    p.write("ignored/tracked.h", "int tracked();\n");
    p.git(&["add", "-f", "ignored/tracked.h"]);
    p.write("ignored/untracked.h", "#error not an input\n");
    p.write("one/drop.hpp", "#error glob excludes this\n");
    p.write("one/two/keep.hpp", "int keep();\n");
    p.write("one/two/build/deep.hpp", "#error directory is excluded\n");
    p.write("lib.rs", "fn rust_only() {}\n");
    p.write("legacy.c", "int c_only(void);\n");
    p.write("module.cppm", "export module ignored;\n");
    p.write("case.C", "int case_sensitive();\n");
    p.write("impl.tpp", "template<class T> T helper(T value);\n");
    p.write(
        "main.cpp",
        "#include \"ignored/tracked.h\"\n#include \"ignored/untracked.h\"\n",
    );

    let report = p.score(".", &["--exclude", "one/*.hpp", "--exclude", "**/build"]);
    assert_selection(
        &report,
        &["case.C", "ignored/tracked.h", "impl.tpp", "main.cpp", "one/two/keep.hpp"],
    );
    assert_eq!(report["selection"]["git_ignores"], true);
    assert_eq!(report["selection"]["mode"], "directory");
    assert_eq!(report["selection"]["excludes"], json!(["one/*.hpp", "**/build"]));
    let directives = report["includes"]["directives"].as_array().unwrap();
    assert_eq!(directives.len(), 2);
    assert_eq!(directives[0]["target"], "ignored/tracked.h");
    assert_eq!(directives[1]["spelling"], "ignored/untracked.h");
    assert_eq!(directives[1]["status"], "external");
    assert_eq!(directives[1]["target"], Value::Null);
}

#[cfg(unix)]
#[test]
fn git_discovery_and_include_lookup_do_not_fail_on_unrelated_directories() {
    let p = Project::new("git-excluded-permission");
    p.git(&["init", "--quiet", "--template="]);
    p.write("project/blocked/a.hpp", "int hidden();\n");
    p.write("project/dep[1].hpp", "int dep();\n");
    p.write("project/main.cpp", "#include \"dep[1].hpp\"\n");
    let _restriction = RestrictedPath::new(p.0.join("project/blocked"));
    if !read_is_denied(&p.0.join("project/blocked/a.hpp")) {
        return;
    }

    let directory = p.score("project", &["--exclude", "blocked/**"]);
    assert_selection(&directory, &["dep[1].hpp", "main.cpp"]);
    assert_eq!(directory["includes"]["directives"][0]["target"], "dep[1].hpp");
    let single = p.score("project/main.cpp", &[]);
    assert_selection(&single, &["project/main.cpp"]);
    assert_eq!(single["includes"]["directives"][0]["target"], "project/dep[1].hpp");

    let incomplete = assert_incomplete(p.run("project", &["scorecard"]));
    assert!(incomplete["diagnostics"].as_array().unwrap().iter().any(|issue| {
        issue["kind"] == "discovery_error"
            && issue["message"].as_str().unwrap().contains("project/blocked/")
    }), "{incomplete}");

    p.write("project/main.cpp", "#include \"blocked/a.hpp\"\n");
    let incomplete = assert_incomplete(p.run("project/main.cpp", &["scorecard"]));
    assert_eq!(incomplete["diagnostics"][0]["kind"], "include_resolution_error");
}

#[cfg(unix)]
#[test]
fn targeted_includes_preserve_git_warnings_that_can_change_eligibility() {
    let p = Project::new("git-include-ignore-permission");
    p.git(&["init", "--quiet", "--template="]);
    p.write("main.cpp", "#include \"deps/api.hpp\"\n");
    p.write("deps/api.hpp", "int api();\n");
    p.write("deps/.gitignore", "/api.hpp\n");
    let _restriction = RestrictedPath::new(p.0.join("deps/.gitignore"));
    if !read_is_denied(&p.0.join("deps/.gitignore")) {
        return;
    }

    let incomplete = assert_incomplete(p.run("main.cpp", &["scorecard"]));
    assert_eq!(incomplete["diagnostics"][0]["kind"], "include_resolution_error");
    assert!(incomplete["diagnostics"][0]["message"].as_str().unwrap().contains("deps/.gitignore"));

    let report = p.score(".", &["--exclude", "deps"]);
    assert_selection(&report, &["main.cpp"]);
    assert_eq!(report["includes"]["directives"][0]["status"], "external");

    // Relative global excludes and a per-directory .gitignore can produce
    // identical warnings, but the global failure can affect every input.
    p.git(&["config", "core.excludesFile", "deps/.gitignore"]);
    let incomplete = assert_incomplete(p.run(".", &["scorecard", "--exclude", "deps"]));
    assert!(incomplete["diagnostics"].as_array().unwrap().iter().any(|issue| {
        issue["kind"] == "discovery_error"
            && issue["message"].as_str().unwrap().contains("deps/.gitignore")
    }), "{incomplete}");
}

#[test]
fn git_include_lookup_does_not_enter_submodules() {
    let p = Project::new("git-include-submodule");
    p.git(&["init", "--quiet", "--template="]);
    p.write("module/api.hpp", "int api();\n");
    p.git(&["-C", "module", "init", "--quiet", "--template="]);
    p.git(&["-C", "module", "add", "api.hpp"]);
    p.git(&["-C", "module", "commit", "--quiet", "-m", "module"]);
    p.git(&["add", "module"]); // Record an index gitlink for the nested worktree.
    p.write("main.cpp", "#include \"module/api.hpp\"\n");

    for path in [".", "main.cpp"] {
        let report = p.score(path, &[]);
        assert_selection(&report, &["main.cpp"]);
        assert_eq!(report["includes"]["directives"][0]["status"], "external");
        assert_eq!(report["includes"]["edges"], json!([]));
    }
}

#[test]
fn changed_scorecards_use_requested_base_and_score_only_existing_eligible_headers() {
    let p = Project::new("changed");
    p.git(&["init", "--quiet", "--template="]);
    p.write(".gitignore", "ignored.hpp\n");
    p.write("src/consumer.cpp", "#include \"unstaged.h\"\nint consumer();\n");
    for file in ["src/staged.hpp", "src/unstaged.h", "src/deleted.hpp", "skip/generated.hpp"] {
        p.write(file, "int before();\n");
    }
    p.git(&["add", "."]);
    p.git(&["commit", "--quiet", "-m", "baseline"]);
    p.git(&["tag", "review-base"]);
    p.write("src/committed.hxx", "int committed();\n");
    p.git(&["add", "src/committed.hxx"]);
    p.git(&["commit", "--quiet", "-m", "committed header"]);

    fs::remove_file(p.0.join("src/deleted.hpp")).unwrap();
    p.write("skip/generated.hpp", "int excluded_update();\n");
    p.write("ignored.hpp", "int ignored_untracked();\n");
    p.write("src/other.rs", "fn unrelated() {}\n");

    let empty = p.score(".", &["--exclude", "skip", "--changed"]);
    assert_eq!(empty["status"], "empty");
    assert_eq!(empty["selection"]["mode"], "changed");
    assert_eq!(empty["selection"]["base"], "HEAD");
    assert_eq!(empty["selected_files"], json!([]));
    assert!(empty.get("code_complexity_cost").is_none());

    p.write("src/staged.hpp", "int in_index();\n");
    p.git(&["add", "src/staged.hpp"]);
    p.write("src/unstaged.h", "#include \"consumer.cpp\"\nint worktree_update();\n");
    p.write("src/untracked file.hh", "int with_space();\n");

    let head = p.score(".", &["--exclude", "skip", "--changed"]);
    assert_selection(&head, &["src/staged.hpp", "src/unstaged.h", "src/untracked file.hh"]);
    assert_eq!(head["selection"]["base"], "HEAD");
    assert_eq!(head["components"]["declaration_surface_cost"], 6);
    assert_eq!(
        head["includes"]["edges"],
        json!([{"source": "src/unstaged.h", "target": "src/consumer.cpp", "cyclic": false}]),
    );
    assert_eq!(head["includes"]["cyclic_components"], json!([]));

    let historical = p.score(".", &["--exclude", "skip", "--changed", "review-base"]);
    assert_selection(
        &historical,
        &["src/committed.hxx", "src/staged.hpp", "src/unstaged.h", "src/untracked file.hh"],
    );
    assert_eq!(historical["selection"]["base"], "review-base");
    assert_eq!(historical["components"]["declaration_surface_cost"], 8);
}

#[cfg(unix)]
#[test]
fn changed_scorecards_fail_closed_for_inaccessible_index_entries() {
    let p = Project::new("git-changed-permission");
    p.git(&["init", "--quiet", "--template="]);
    p.write(".gitignore", "/blocked/\n");
    for file in ["blocked/staged.hpp", "blocked/unstaged.hpp"] {
        p.write(file, "int before();\n");
        p.git(&["add", "-f", file]);
    }
    p.git(&["add", ".gitignore"]);
    p.git(&["commit", "--quiet", "-m", "baseline"]);
    p.write("blocked/staged.hpp", "int staged();\n");
    p.git(&["add", "-f", "blocked/staged.hpp"]);
    p.write("blocked/unstaged.hpp", "int unstaged();\n");

    let _restriction = RestrictedPath::new(p.0.join("blocked"));
    if !read_is_denied(&p.0.join("blocked/unstaged.hpp")) {
        return;
    }
    let report = assert_incomplete(p.run(".", &["scorecard", "--changed"]));
    let diagnostics = report["diagnostics"].as_array().unwrap();
    for file in ["blocked/staged.hpp", "blocked/unstaged.hpp"] {
        assert!(diagnostics.iter().any(|issue| {
            issue["file"] == file && issue["kind"] == "discovery_error"
        }), "{report}");
    }
    let excluded = p.score(".", &["--changed", "--exclude", "blocked"]);
    assert_eq!(excluded["status"], "empty", "{excluded}");
    assert_eq!(excluded["diagnostics"], json!([]));
}

#[test]
fn changed_scorecards_skip_sparse_absences_and_ignored_staged_removals() {
    let p = Project::new("git-changed-sparse");
    p.git(&["init", "--quiet", "--template="]);
    p.write(".gitignore", "/ignored.hpp\n");
    p.write("ignored.hpp", "int ignored();\n");
    p.write("visible/unchanged.hpp", "int unchanged();\n");
    p.write("sparse/changed.hpp", "int before();\n");
    p.git(&["add", "."]);
    p.git(&["add", "-f", "ignored.hpp"]);
    p.git(&["commit", "--quiet", "-m", "baseline"]);
    p.git(&["tag", "baseline"]);
    p.write("sparse/changed.hpp", "int after();\n");
    p.git(&["add", "sparse/changed.hpp"]);
    p.git(&["commit", "--quiet", "-m", "change sparse file"]);
    p.git(&["sparse-checkout", "set", "--cone", "visible"]);
    p.git(&["rm", "--cached", "ignored.hpp"]);
    assert!(!p.0.join("sparse/changed.hpp").exists());
    assert!(p.0.join("ignored.hpp").is_file());

    let report = p.score(".", &["--changed", "baseline"]);
    assert_eq!(report["status"], "empty", "{report}");
    assert_eq!(report["selected_files"], json!([]));
    assert_eq!(report["diagnostics"], json!([]));
}

#[cfg(unix)]
#[test]
fn hardlinked_include_candidates_and_discovered_inputs_count_once() {
    let p = Project::new("hardlinks");
    p.write("main.cpp", "#include <shared.h>\n");
    p.write("inc/shared.h", "int shared();\n");
    fs::create_dir(p.0.join("other")).unwrap();
    fs::hard_link(p.0.join("inc/shared.h"), p.0.join("other/shared.h")).unwrap();

    let report = p.score(".", &["--include-dir", "other", "--include-dir", "inc"]);
    assert_selection(&report, &["inc/shared.h", "main.cpp"]);
    let graph = &report["includes"];
    assert_eq!(graph["directives"][0]["status"], "internal");
    assert_eq!(graph["directives"][0]["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(
        graph["edges"],
        json!([{"source": "main.cpp", "target": "inc/shared.h", "cyclic": false}]),
    );
    assert_eq!(report["components"]["declaration_surface_cost"], 2);
}

#[cfg(unix)]
#[test]
fn includes_apply_excludes_after_resolving_parent_traversal() {
    let p = Project::new("include-symlink-parent");
    p.write("main.cpp", "#include \"link/../excluded/api.hpp\"\n");
    p.write("real/excluded/api.hpp", "int api();\n");
    fs::create_dir(p.0.join("real/child")).unwrap();
    fs::create_dir(p.0.join("excluded")).unwrap();
    std::os::unix::fs::symlink("real/child", p.0.join("link")).unwrap();

    let report = p.score(".", &["--exclude", "excluded"]);
    assert_selection(&report, &["main.cpp", "real/excluded/api.hpp"]);
    assert_eq!(
        report["includes"]["edges"],
        json!([{"source": "main.cpp", "target": "real/excluded/api.hpp", "cyclic": false}]),
    );
    assert_eq!(report["includes"]["directives"][0]["status"], "internal");
    assert_eq!(report["components"]["include_coupling_cost"], 2);

    for exclude in ["link", "real/excluded"] {
        let report = p.score(".", &["--exclude", exclude]);
        assert_eq!(report["includes"]["directives"][0]["status"], "external");
        assert_eq!(report["includes"]["edges"], json!([]));
        assert_eq!(report["components"]["include_coupling_cost"], 0);
    }

    p.git(&["init", "--quiet", "--template="]);
    p.write(".gitignore", "/link\n");
    let ignored_alias = p.score("main.cpp", &[]);
    assert_eq!(ignored_alias["includes"]["directives"][0]["status"], "external");

    p.git(&["add", "-f", "link"]);
    let tracked_alias = p.score("main.cpp", &[]);
    assert_eq!(tracked_alias["includes"]["directives"][0]["target"], "real/excluded/api.hpp");
}

#[cfg(unix)]
#[test]
fn explicit_file_selection_filters_the_requested_symlink_name() {
    let p = Project::new("explicit-symlink");
    p.write("real.hpp", "int measured() { return 0; }\n");
    std::os::unix::fs::symlink("real.hpp", p.0.join("alias.hpp")).unwrap();
    assert_selection(&p.score("alias.hpp", &[]), &["alias.hpp"]);

    let assert_empty = |args: &[&str], success| {
        let output = p.run("alias.hpp", args);
        assert_eq!(output.status.success(), success, "{output:?}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["status"], "empty", "{report}");
        assert_eq!(report["selected_files"], json!([]));
        assert!(report.get("code_complexity_cost").is_none());
    };
    assert_empty(&["scorecard", "--exclude", "alias.hpp"], false);
    assert_empty(&["scorecard", "--exclude", "real.hpp"], false);

    p.git(&["init", "--quiet", "--template="]);
    p.write(".gitignore", "/alias.hpp\n");
    p.git(&["add", ".gitignore", "real.hpp"]);
    p.git(&["commit", "--quiet", "-m", "baseline"]);
    p.write("real.hpp", "int measured() { return 1; }\n");
    assert_empty(&["scorecard"], false);
    assert_empty(&["loc"], false);
    assert_empty(&["scorecard", "--changed"], true);
    assert_selection(&p.score("real.hpp", &["--changed"]), &["real.hpp"]);
}
