//! End-to-end scenario tests for `cstat extract` against the §13 fixture
//! and the unparseable-file contract fixture.
//!
//! These observe the JSON envelope only — they should survive any refactor
//! that preserves the spec §7 contract.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use cstat_schema::{LevelStatus, OracleOutput};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root resolves")
}

fn run_extract(project: &Path) -> OracleOutput {
    run_extract_with(project, &[])
}

fn run_extract_with(project: &Path, extra_args: &[&str]) -> OracleOutput {
    let bin = env!("CARGO_BIN_EXE_cstat");
    let mut cmd = Command::new(bin);
    cmd.args(["extract", "--path"]).arg(project);
    cmd.args(extra_args);
    let output = cmd.output().expect("invoke cstat binary");

    assert!(
        output.status.success(),
        "cstat extract on {project:?} failed: status={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).expect("stdout parses as OracleOutput envelope")
}

/// Default (codegen-disabled) extract on the §13 fixture. Cached so the
/// many scenario tests that share this baseline don't each pay the
/// cost of the external-tool stages (clippy, rustdoc, cargo test).
fn run_extract_on_fixture() -> OracleOutput {
    static CACHE: OnceLock<OracleOutput> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let fixture = workspace_root().join("examples").join("single-file-lib");
            assert!(fixture.is_dir(), "fixture missing: {fixture:?}");
            run_extract(&fixture)
        })
        .clone()
}

/// Shared codegen-enabled extract. Cached so the tests don't trigger
/// repeated cargo builds (each ~3s) or contend on the fixture's
/// `target/cstat-*` lock when run in parallel.
fn run_extract_on_fixture_with_codegen() -> &'static OracleOutput {
    static CACHE: OnceLock<OracleOutput> = OnceLock::new();
    CACHE.get_or_init(|| {
        let fixture = workspace_root().join("examples").join("single-file-lib");
        assert!(fixture.is_dir(), "fixture missing: {fixture:?}");
        run_extract_with(&fixture, &["--enable-codegen"])
    })
}

fn run_extract_on_broken_fixture() -> OracleOutput {
    let fixture = workspace_root().join("examples").join("broken-file");
    assert!(fixture.is_dir(), "fixture missing: {fixture:?}");
    run_extract(&fixture)
}

/// Shared extract on the `with-impls` fixture (lib with traits, an
/// inherent impl, two trait-impls, and a `Display` impl exercising the
/// orphan-rule quadrants). Cached so we don't shell out repeatedly.
fn run_extract_on_with_impls_fixture() -> &'static OracleOutput {
    static CACHE: OnceLock<OracleOutput> = OnceLock::new();
    CACHE.get_or_init(|| {
        let fixture = workspace_root().join("examples").join("with-impls");
        assert!(fixture.is_dir(), "fixture missing: {fixture:?}");
        run_extract(&fixture)
    })
}

/// Shared extract on the `with-graphs` fixture (lib with three modules
/// and a small intra-crate call graph), used by the Stage 9 graph tests.
/// Cached so we don't shell out repeatedly.
fn run_extract_on_with_graphs_fixture() -> &'static OracleOutput {
    static CACHE: OnceLock<OracleOutput> = OnceLock::new();
    CACHE.get_or_init(|| {
        let fixture = workspace_root().join("examples").join("with-graphs");
        assert!(fixture.is_dir(), "fixture missing: {fixture:?}");
        run_extract(&fixture)
    })
}

/// Stage-3 levels (spec §2.1 1–6 plus §2.9 #47) — always `ok` on the fixture.
const STAGE3_OK_LEVELS: &[&str] = &[
    "fs.layout",
    "fs.bytes",
    "fs.source",
    "tok.stream",
    "tok.comments",
    "fmt.whitespace",
    "rustfmt",
];

/// Stage-4 levels that always report `ok` on a well-formed project.
/// Level 9 (`ast.expanded`) is conditional on cargo-expand and exercised
/// separately.
const STAGE4_OK_LEVELS: &[&str] = &["ast.cst", "ast.items"];

/// Stage-5 levels — spec §2.8 41–44. `cargo.lock` is conditional on a
/// generated `Cargo.lock` (which the fixture commits, so it's `ok` there).
const STAGE5_OK_LEVELS: &[&str] = &[
    "cargo.workspace",
    "cargo.lock",
    "cargo.cfg",
    "cargo.toolchain",
];

#[test]
fn every_required_level_is_present_with_a_documented_status() {
    let envelope = run_extract_on_fixture();
    let required_ids: Vec<&str> = cstat_orchestrate::REQUIRED_LEVELS
        .iter()
        .map(|d| d.id)
        .collect();

    assert_eq!(
        envelope.levels.len(),
        required_ids.len(),
        "envelope levels count differs from required-levels table"
    );

    for id in required_ids {
        let report = envelope
            .levels
            .get(id)
            .unwrap_or_else(|| panic!("level {id} missing from envelope"));
        match report.status {
            LevelStatus::Ok => {}
            LevelStatus::Skipped | LevelStatus::Failed => {
                assert!(
                    report.reason.as_ref().is_some_and(|r| !r.is_empty()),
                    "non-ok level {id} has empty reason"
                );
            }
        }
    }
}

#[test]
fn envelope_carries_a_populated_toolchain_stamp_and_schema_version() {
    let envelope = run_extract_on_fixture();
    assert!(!envelope.schema_version.is_empty());
    assert!(!envelope.cstat_version.is_empty());
    assert!(!envelope.toolchain.version.is_empty(), "toolchain version");
    assert!(!envelope.toolchain.channel.is_empty(), "toolchain channel");
}

#[test]
fn meta_levels_missing_count_matches_non_ok_level_count() {
    let envelope = run_extract_on_fixture();
    let observed_non_ok = envelope
        .levels
        .values()
        .filter(|r| r.status != LevelStatus::Ok)
        .count();

    let missing = envelope
        .vector
        .get("meta.levels_missing.count")
        .expect("meta.levels_missing.count present");

    assert_eq!(
        missing as usize, observed_non_ok,
        "meta counter must agree with the levels map"
    );
}

#[test]
fn stage_three_levels_report_ok_on_the_fixture() {
    let envelope = run_extract_on_fixture();
    for id in STAGE3_OK_LEVELS {
        let report = envelope
            .levels
            .get(*id)
            .unwrap_or_else(|| panic!("level {id} missing"));
        assert_eq!(
            report.status,
            LevelStatus::Ok,
            "stage-3 level {id} expected ok but got {:?} (reason: {:?})",
            report.status,
            report.reason
        );
    }
}

#[test]
fn fixture_vector_contains_the_completion_gate_metrics() {
    let envelope = run_extract_on_fixture();

    assert_eq!(
        envelope.vector.get("fs.global.rust_file_count"),
        Some(1.0),
        "fixture has one .rs file"
    );
    let total_bytes = envelope
        .vector
        .get("fs.global.total_bytes")
        .expect("fs.global.total_bytes populated");
    assert!(total_bytes > 0.0, "total_bytes must reflect the .rs payload");

    for key in [
        "fs.layout.file_size_bytes.max",
        "fs.layout.file_size_bytes.mean",
    ] {
        let v = envelope
            .vector
            .get(key)
            .unwrap_or_else(|| panic!("{key} missing"));
        assert!(v >= 0.0, "{key} must be non-negative");
    }

    let blank_ratio = envelope
        .vector
        .get("fs.source.blank_line_ratio.mean")
        .expect("blank_line_ratio.mean populated");
    assert!(
        (0.0..=1.0).contains(&blank_ratio),
        "blank_line_ratio must be a fraction, got {blank_ratio}"
    );

    let unique_idents = envelope
        .vector
        .get("tok.global.unique_identifiers")
        .expect("tok.global.unique_identifiers populated");
    assert!(
        unique_idents >= 5.0,
        "fixture should have several distinct identifiers, got {unique_idents}"
    );

    assert_eq!(envelope.vector.get("tok.comments.todo_count"), Some(0.0));
    assert_eq!(envelope.vector.get("tok.comments.fixme_count"), Some(0.0));

    let conformant = envelope
        .vector
        .get("rustfmt.conformant")
        .expect("rustfmt.conformant populated");
    assert!(
        conformant == 0.0 || conformant == 1.0,
        "rustfmt.conformant is a 0/1 flag, got {conformant}"
    );
}

#[test]
fn stage_four_ast_levels_report_ok_on_the_fixture() {
    let envelope = run_extract_on_fixture();
    for id in STAGE4_OK_LEVELS {
        let report = envelope
            .levels
            .get(*id)
            .unwrap_or_else(|| panic!("level {id} missing"));
        assert_eq!(
            report.status,
            LevelStatus::Ok,
            "stage-4 level {id} expected ok but got {:?} (reason: {:?})",
            report.status,
            report.reason
        );
    }
}

#[test]
fn ast_expanded_is_ok_or_skipped_with_a_reason() {
    let envelope = run_extract_on_fixture();
    let report = envelope.levels.get("ast.expanded").expect("ast.expanded present");
    match report.status {
        LevelStatus::Ok => {
            assert!(
                envelope.vector.contains_key("ast.expanded.token_count"),
                "ok ast.expanded must populate its vector keys"
            );
        }
        LevelStatus::Skipped => {
            assert!(
                report.reason.as_ref().is_some_and(|r| !r.is_empty()),
                "skipped ast.expanded must explain itself"
            );
            assert!(
                !envelope.vector.contains_key("ast.expanded.token_count"),
                "skipped levels must omit their vector keys (spec §8)"
            );
        }
        LevelStatus::Failed => panic!(
            "ast.expanded failed on a well-formed fixture (reason: {:?})",
            report.reason
        ),
    }
}

#[test]
fn fixture_ast_metrics_reflect_the_two_pub_functions() {
    let envelope = run_extract_on_fixture();

    // §13: two top-level `pub fn`s. The exact value is the §13 anchor.
    assert_eq!(
        envelope.vector.get("ast.global.func_count"),
        Some(2.0),
        "fixture has exactly two functions"
    );

    let parsed = envelope
        .vector
        .get("ast.cst.parsed_file_count")
        .expect("ast.cst.parsed_file_count populated");
    assert_eq!(parsed, 1.0, "single-file fixture parses one file");

    let pub_ratio = envelope
        .vector
        .get("ast.items.pub_ratio")
        .expect("ast.items.pub_ratio populated");
    assert!(
        (0.0..=1.0).contains(&pub_ratio) && pub_ratio > 0.0,
        "fixture's items are all `pub`, ratio must be > 0"
    );

    // The crate-root module must show up under entities.ast.module with the
    // function count it owns.
    let modules = envelope
        .entities
        .get("ast.module")
        .expect("entities.ast.module populated");
    assert!(
        modules
            .iter()
            .any(|r| r.metrics.get("func_count").copied() == Some(2.0)),
        "expected a module with two functions; got {modules:?}"
    );
}

#[test]
fn meta_files_unparseable_count_is_zero_on_the_fixture() {
    let envelope = run_extract_on_fixture();
    assert_eq!(
        envelope.vector.get("meta.files_unparseable.count"),
        Some(0.0),
        "well-formed fixture has no parse errors"
    );
    assert!(
        envelope.entities.get("ast.file_errors").map_or(true, |v| v.is_empty()),
        "well-formed fixture must not produce file_errors entries"
    );
}

#[test]
fn unparseable_file_degrades_gracefully_per_spec_section_11() {
    // Anchor test 6 (§11 row 4): one well-formed file + one broken file.
    let envelope = run_extract_on_broken_fixture();

    // The process exits 0 — already asserted by run_extract on success.

    // AST levels still report ok despite one file failing to parse.
    for id in ["ast.cst", "ast.items"] {
        let report = envelope.levels.get(id).expect("ast level present");
        assert_eq!(
            report.status,
            LevelStatus::Ok,
            "{id} must stay ok in the face of one bad file (reason: {:?})",
            report.reason
        );
    }

    // The broken file shows up under entities.ast.file_errors.
    let errors = envelope
        .entities
        .get("ast.file_errors")
        .expect("entities.ast.file_errors populated");
    assert_eq!(
        errors.len(),
        1,
        "exactly one unparseable file expected, got {}",
        errors.len()
    );
    let bad = &errors[0];
    let file_id = bad
        .ids
        .file
        .as_ref()
        .expect("file_errors entry carries a FileId");
    assert!(
        file_id.as_str().ends_with("bad.rs"),
        "file_errors should identify bad.rs, got {file_id:?}"
    );

    // meta counter agrees with the entity record count.
    assert_eq!(
        envelope.vector.get("meta.files_unparseable.count"),
        Some(1.0),
        "meta.files_unparseable.count must reflect entities.ast.file_errors"
    );

    // The good file's metrics still land: the surviving `pub fn good` shows up.
    assert_eq!(
        envelope.vector.get("ast.global.func_count"),
        Some(1.0),
        "good.rs's single function must still be counted"
    );
    assert_eq!(
        envelope.vector.get("ast.cst.parsed_file_count"),
        Some(1.0),
        "exactly one .rs file parsed successfully"
    );
    assert_eq!(
        envelope.vector.get("ast.cst.unparseable_file_count"),
        Some(1.0),
        "exactly one .rs file failed to parse"
    );
}

#[test]
fn stage_five_cargo_metadata_levels_report_ok_on_the_fixture() {
    let envelope = run_extract_on_fixture();
    for id in STAGE5_OK_LEVELS {
        let report = envelope
            .levels
            .get(*id)
            .unwrap_or_else(|| panic!("level {id} missing"));
        assert_eq!(
            report.status,
            LevelStatus::Ok,
            "stage-5 level {id} expected ok but got {:?} (reason: {:?})",
            report.status,
            report.reason
        );
    }
}

#[test]
fn fixture_cargo_metadata_reflects_a_single_crate_with_no_deps() {
    let envelope = run_extract_on_fixture();

    // §13 fixture is a single package with no dependencies.
    assert_eq!(
        envelope.vector.get("cargo.workspace.member_count"),
        Some(1.0),
        "fixture is a single crate"
    );
    assert_eq!(
        envelope.vector.get("cargo.workspace.direct_deps"),
        Some(0.0),
        "fixture declares no dependencies"
    );
    assert_eq!(
        envelope.vector.get("cargo.workspace.dev_deps"),
        Some(0.0),
        "fixture declares no dev-dependencies"
    );
    assert_eq!(
        envelope.vector.get("cargo.workspace.edition"),
        Some(2021.0),
        "fixture is edition 2021"
    );
    assert_eq!(
        envelope.vector.get("cargo.workspace.is_virtual"),
        Some(0.0),
        "fixture is a real package, not a virtual workspace"
    );

    // Cargo.lock has one entry: the package itself, with no resolved deps.
    assert_eq!(
        envelope.vector.get("cargo.lock.package_count"),
        Some(1.0),
        "fixture lockfile has one package entry"
    );
    let edge_count = envelope
        .vector
        .get("cargo.lock.dep_edge_count")
        .expect("cargo.lock.dep_edge_count populated");
    assert_eq!(edge_count, 0.0, "fixture has no resolved deps");
    let lockfile_version = envelope
        .vector
        .get("cargo.lock.lockfile_version")
        .expect("cargo.lock.lockfile_version populated");
    assert!(lockfile_version >= 3.0, "modern cargo writes v3+ lockfiles");

    // rustc --print=cfg should yield at least a handful of entries on any
    // supported host. Exact contents vary, so we only assert "non-empty".
    let cfg_count = envelope
        .vector
        .get("cargo.cfg.entry_count")
        .expect("cargo.cfg.entry_count populated");
    assert!(
        cfg_count > 0.0,
        "rustc --print=cfg returned no entries (got {cfg_count})"
    );

    // Toolchain pin: fixture has no rust-toolchain.toml.
    assert_eq!(
        envelope.vector.get("cargo.toolchain.pinned"),
        Some(0.0),
        "fixture does not pin a toolchain"
    );
}

#[test]
fn envelope_toolchain_stamp_classifies_the_active_channel() {
    let envelope = run_extract_on_fixture();
    let channel = envelope.toolchain.channel.as_str();
    assert!(
        matches!(channel, "stable" | "beta" | "nightly" | "dev"),
        "channel {channel:?} is not one of the documented values"
    );
    assert!(!envelope.toolchain.version.is_empty());
    assert!(!envelope.toolchain.host.is_empty());
}

#[test]
fn provenance_attributes_each_vector_key_to_exactly_one_level() {
    let envelope = run_extract_on_fixture();
    let level_ids: BTreeSet<&str> = cstat_orchestrate::REQUIRED_LEVELS
        .iter()
        .map(|d| d.id)
        .chain(std::iter::once("meta"))
        .collect();

    for (key, _) in envelope.vector.iter() {
        let owners = envelope
            .provenance
            .0
            .get(key)
            .unwrap_or_else(|| panic!("no provenance entry for vector key {key}"));
        assert_eq!(
            owners.len(),
            1,
            "vector key {key} has multiple owners {owners:?}; namespace cross-check failed"
        );
        let owner = owners[0].as_str();
        assert!(
            level_ids.contains(owner),
            "vector key {key} attributed to unknown level {owner}"
        );
    }
}

// ───── Stage 6: codegen extractors ──────────────────────────────────────

/// The five codegen-level ids covered by Stage 6 (spec §2.5 20–24).
const STAGE6_LEVELS: &[&str] = &["llvm.preopt", "llvm.opt", "asm", "obj", "bin"];

#[test]
fn stage_six_codegen_levels_skip_cleanly_without_the_flag() {
    // Default `cstat extract` (no --enable-codegen) must not drive a
    // cargo build. All five Stage-6 levels report Skipped with a reason
    // that explains how to turn them on. Spec §8 omission contract: no
    // codegen vector keys leak into the envelope.
    let envelope = run_extract_on_fixture();
    for id in STAGE6_LEVELS {
        let report = envelope.levels.get(*id).expect("level present");
        assert_eq!(
            report.status,
            LevelStatus::Skipped,
            "{id} should skip when codegen is disabled; got {:?}",
            report.status
        );
        let reason = report.reason.as_deref().unwrap_or("");
        assert!(
            !reason.is_empty(),
            "{id} skipped without a reason — user has no way to know how to enable it"
        );
    }
    for key in envelope.vector.0.keys() {
        for prefix in ["llvm.preopt.", "llvm.opt.", "asm.", "obj.", "bin."] {
            assert!(
                !key.starts_with(prefix),
                "skipped codegen level emitted {key} (spec §8 says skipped levels omit their keys)"
            );
        }
    }
}

#[test]
fn stage_six_codegen_levels_report_ok_when_enabled_on_the_fixture() {
    let envelope = run_extract_on_fixture_with_codegen();
    // The fixture is lib-only, so bin legitimately skips. Every other
    // Stage-6 level must report ok.
    for id in ["llvm.preopt", "llvm.opt", "asm", "obj"] {
        let report = envelope.levels.get(id).expect("level present");
        assert_eq!(
            report.status,
            LevelStatus::Ok,
            "{id} expected ok with --enable-codegen; got {:?} (reason: {:?})",
            report.status,
            report.reason
        );
    }
    let bin_report = envelope.levels.get("bin").expect("bin level present");
    assert_eq!(
        bin_report.status,
        LevelStatus::Skipped,
        "bin should skip on the lib-only fixture; got {:?} (reason: {:?})",
        bin_report.status,
        bin_report.reason
    );
    let bin_reason = bin_report.reason.as_deref().unwrap_or("");
    assert!(
        bin_reason.contains("bin") || bin_reason.contains("link"),
        "bin skip reason should explain why; got {bin_reason:?}"
    );
}

#[test]
fn fixture_codegen_per_function_records_show_add_and_classify() {
    let envelope = run_extract_on_fixture_with_codegen();

    // The fixture defines exactly two functions. Both should appear in
    // the per-function LLVM IR records, pre and post optimization.
    for kind in ["llvm.preopt_func", "llvm.opt_func"] {
        let records = envelope
            .entities
            .get(kind)
            .unwrap_or_else(|| panic!("entities.{kind} missing"));
        assert_eq!(
            records.len(),
            2,
            "fixture defines two functions but {kind} has {} records",
            records.len()
        );
        for r in records {
            assert!(
                r.metrics.values().all(|v| v.is_finite() && *v >= 0.0),
                "all per-function metrics must be finite non-negative; got {:?}",
                r.metrics
            );
        }
    }

    // Summary scalars must agree with the per-function records.
    let summary_count = envelope
        .vector
        .get("llvm.preopt.function_count")
        .expect("llvm.preopt.function_count populated");
    assert_eq!(summary_count, 2.0);

    let bb_sum = envelope
        .vector
        .get("llvm.preopt.basic_block_count.sum")
        .expect("llvm.preopt.basic_block_count.sum populated");
    let per_func_bb_sum: f64 = envelope
        .entities
        .get("llvm.preopt_func")
        .unwrap()
        .iter()
        .map(|r| r.metrics.get("preopt_basic_blocks").copied().unwrap_or(0.0))
        .sum();
    assert_eq!(
        bb_sum, per_func_bb_sum,
        "vector sum must equal sum-over-entities"
    );

    // Assembly: the fixture has two functions, so the .s file should
    // declare at least two function symbols.
    let asm_funcs = envelope
        .vector
        .get("asm.global.function_count")
        .expect("asm.global.function_count populated");
    assert!(
        asm_funcs >= 2.0,
        "assembly should declare ≥ 2 functions, got {asm_funcs}"
    );
}

#[test]
fn fixture_codegen_object_section_artifacts_are_observed() {
    let envelope = run_extract_on_fixture_with_codegen();
    // The fixture is lib-only, so its build artifact is an .rlib. The
    // obj level should observe at least one rlib with non-trivial size.
    let rlib_count = envelope
        .vector
        .get("obj.global.rlib_file_count")
        .expect("obj.global.rlib_file_count populated");
    assert!(rlib_count >= 1.0, "expected at least one rlib, got {rlib_count}");

    let rlib_bytes = envelope
        .vector
        .get("obj.global.rlib_total_bytes")
        .expect("obj.global.rlib_total_bytes populated");
    assert!(rlib_bytes > 0.0, "rlib should not be empty");
}

// ───── Stage 7: binary-artifact extractors + AST↔Symbol join ────────────

/// The six binary-artifact level ids covered by Stage 7 (spec §2.6 25–30).
const STAGE7_LEVELS: &[&str] = &[
    "sym.table",
    "sym.demangled",
    "sym.sections",
    "sym.relocs",
    "dwarf",
    "link.map",
];

#[test]
fn stage_seven_binary_levels_skip_cleanly_without_codegen() {
    // Without --enable-codegen there are no object artifacts to read,
    // so every Stage 7 level must skip with a reason and emit no keys.
    let envelope = run_extract_on_fixture();
    for id in STAGE7_LEVELS {
        let report = envelope.levels.get(*id).expect("level present");
        assert_eq!(
            report.status,
            LevelStatus::Skipped,
            "{id} should skip without codegen; got {:?}",
            report.status
        );
        let reason = report.reason.as_deref().unwrap_or("");
        assert!(!reason.is_empty(), "{id} skipped without a reason");
    }
    for key in envelope.vector.0.keys() {
        for prefix in ["sym.", "dwarf.", "link.map."] {
            assert!(
                !key.starts_with(prefix),
                "skipped binary level emitted {key} (spec §8 says skipped levels omit their keys)"
            );
        }
    }
}

#[test]
fn stage_seven_binary_levels_report_ok_with_codegen_on_the_lib_fixture() {
    let envelope = run_extract_on_fixture_with_codegen();
    // sym.table, sym.demangled, sym.sections, sym.relocs, dwarf must
    // all be ok on the fixture (which has a real .rlib with DWARF).
    // link.map legitimately skips: no -Wl,-Map flag was passed.
    for id in ["sym.table", "sym.demangled", "sym.sections", "sym.relocs", "dwarf"] {
        let report = envelope.levels.get(id).expect("level present");
        assert_eq!(
            report.status,
            LevelStatus::Ok,
            "{id} expected ok with codegen; got {:?} (reason: {:?})",
            report.status,
            report.reason
        );
    }
    let link = envelope.levels.get("link.map").expect("link.map present");
    assert_eq!(
        link.status,
        LevelStatus::Skipped,
        "link.map should skip on the lib-only fixture; got {:?}",
        link.status
    );
    let reason = link.reason.as_deref().unwrap_or("");
    assert!(
        reason.contains("link") || reason.contains("Map"),
        "link.map skip reason should mention linking; got {reason:?}"
    );
}

#[test]
fn fixture_symbol_table_reports_at_least_the_two_pub_functions() {
    let envelope = run_extract_on_fixture_with_codegen();
    let func_count = envelope
        .vector
        .get("sym.global.function_count")
        .expect("sym.global.function_count populated");
    assert!(
        func_count >= 2.0,
        "fixture's two pub fns must produce ≥ 2 function symbols, got {func_count}"
    );

    // Per-function size distribution: max ≥ 0, mean ≥ 0, sum ≥ 0.
    for key in [
        "sym.func.size_bytes.max",
        "sym.func.size_bytes.mean",
        "sym.func.size_bytes.sum",
    ] {
        let v = envelope.vector.get(key).unwrap_or_else(|| panic!("{key} missing"));
        assert!(v.is_finite() && v >= 0.0, "{key} must be a finite non-negative; got {v}");
    }

    // Per-symbol entity records: one per function symbol with a SymbolId.
    let records = envelope
        .entities
        .get("sym.func")
        .expect("entities.sym.func populated");
    assert!(
        records.len() >= 2,
        "expected ≥ 2 function-symbol records, got {}",
        records.len()
    );
    for r in records {
        assert!(
            r.ids.symbol.is_some(),
            "every sym.func record must carry a SymbolId; got {r:?}"
        );
    }
}

#[test]
fn fixture_section_layout_observes_text_and_debug_bytes() {
    let envelope = run_extract_on_fixture_with_codegen();
    let text = envelope
        .vector
        .get("sym.sections.text_bytes")
        .expect("sym.sections.text_bytes populated");
    assert!(text > 0.0, "fixture's two fns must produce some .text");
    let debug = envelope
        .vector
        .get("sym.sections.debug_bytes")
        .expect("sym.sections.debug_bytes populated");
    assert!(debug > 0.0, "dev-profile build must carry DWARF debug sections");
    let text_ratio = envelope
        .vector
        .get("sym.sections.text_ratio")
        .expect("text_ratio populated");
    assert!(
        (0.0..=1.0).contains(&text_ratio),
        "text_ratio must be a fraction; got {text_ratio}"
    );
}

#[test]
fn fixture_dwarf_subprograms_match_the_two_pub_functions() {
    let envelope = run_extract_on_fixture_with_codegen();
    let n = envelope
        .vector
        .get("dwarf.global.subprogram_count")
        .expect("dwarf.global.subprogram_count populated");
    assert!(
        n >= 2.0,
        "fixture's two pub fns must show up as DWARF subprograms; got {n}"
    );
    let with_link = envelope
        .vector
        .get("dwarf.global.subprograms_with_linkage_count")
        .expect("subprograms_with_linkage_count populated");
    assert!(with_link >= 2.0, "both fns must have DW_AT_linkage_name");
}

/// §13 worked-example anchor: the `classify` entity has both an
/// `AstFuncId` and a `SymbolId`. The DWARF level is what joins them
/// (via DW_AT_name → DW_AT_linkage_name).
#[test]
fn fixture_ast_symbol_join_populates_both_ids_on_classify() {
    let envelope = run_extract_on_fixture_with_codegen();
    let records = envelope
        .entities
        .get("ast.func")
        .expect("entities.ast.func populated by the DWARF level");
    let classify = records
        .iter()
        .find(|r| {
            r.ids
                .ast_func
                .as_ref()
                .is_some_and(|id| id.fq_path.ends_with("::classify"))
        })
        .expect("classify entity present in entities.ast.func");
    // Spec §6 / §13: both IDs populated.
    assert!(
        classify.ids.ast_func.is_some(),
        "classify must carry AstFuncId"
    );
    let symbol = classify
        .ids
        .symbol
        .as_ref()
        .expect("classify must carry SymbolId after the DWARF join");
    assert!(
        symbol.as_str().contains("classify"),
        "joined SymbolId must mention the source name; got {symbol:?}"
    );

    // The join's success rate scalar should reflect that both fns
    // joined.
    let success = envelope
        .vector
        .get("dwarf.global.join_success_count")
        .expect("join_success_count populated");
    let attempts = envelope
        .vector
        .get("dwarf.global.join_attempt_count")
        .expect("join_attempt_count populated");
    assert!(
        attempts >= 2.0,
        "two AST fns means at least two join attempts; got {attempts}"
    );
    assert!(
        success >= 2.0,
        "both fixture fns have unambiguous DWARF names; got success={success}"
    );
}

#[test]
fn fixture_source_to_symbol_ratio_is_one_with_no_generics() {
    // Spec §13: source_to_symbol_ratio = 1.0 when no generics are
    // monomorphized. The fixture has no generics.
    let envelope = run_extract_on_fixture_with_codegen();
    let ratio = envelope
        .vector
        .get("sym.global.source_to_symbol_ratio")
        .expect("sym.global.source_to_symbol_ratio populated");
    assert!(
        (ratio - 1.0).abs() < 1e-9,
        "expected source-to-symbol ratio = 1.0 (no generics); got {ratio}"
    );
    let generics = envelope
        .vector
        .get("sym.demangled.generic_instantiation_count")
        .expect("generic_instantiation_count populated");
    assert_eq!(generics, 0.0, "fixture has no generics");
}

// ───── Stage 8: type-system extractors (stable side) ────────────────────

/// All three Stage 8 level ids — `type.usage` (AST approximation),
/// `type.trait_impl`, and `type.coherence` (rustdoc-JSON-derived).
const STAGE8_LEVELS: &[&str] = &["type.usage", "type.trait_impl", "type.coherence"];

#[test]
fn stage_eight_levels_appear_with_a_documented_status() {
    let envelope = run_extract_on_fixture();
    for id in STAGE8_LEVELS {
        let report = envelope
            .levels
            .get(*id)
            .unwrap_or_else(|| panic!("level {id} missing"));
        match report.status {
            LevelStatus::Ok => {}
            LevelStatus::Skipped | LevelStatus::Failed => {
                assert!(
                    report.reason.as_ref().is_some_and(|r| !r.is_empty()),
                    "non-ok stage-8 level {id} has empty reason"
                );
            }
        }
    }
}

#[test]
fn fixture_type_usage_observes_the_two_pub_functions() {
    // type.usage is AST-derived, so it must be `ok` on every well-formed
    // fixture — no nightly prerequisite.
    let envelope = run_extract_on_fixture();
    let report = envelope.levels.get("type.usage").expect("type.usage present");
    assert_eq!(
        report.status,
        LevelStatus::Ok,
        "type.usage is AST-derived; should be ok (reason: {:?})",
        report.reason
    );

    assert_eq!(
        envelope.vector.get("type.usage.function_count"),
        Some(2.0),
        "fixture has exactly two functions"
    );
    // Both functions reference `i32`; `classify` also uses `str`. Distinct
    // types overall is therefore at least 2.
    let overall = envelope
        .vector
        .get("type.usage.distinct_types_overall")
        .expect("type.usage.distinct_types_overall populated");
    assert!(overall >= 2.0, "expected ≥ 2 distinct types overall, got {overall}");
    let primitives = envelope
        .vector
        .get("type.usage.primitive_types_overall")
        .expect("primitive_types_overall populated");
    assert!(primitives >= 1.0, "expected i32 to register as a primitive");

    // Per-entity records: one per function, each with an AstFuncId.
    let records = envelope
        .entities
        .get("type.usage_func")
        .expect("entities.type.usage_func populated");
    assert_eq!(records.len(), 2, "two functions, two records");
    for r in records {
        assert!(
            r.ids.ast_func.is_some(),
            "type.usage_func record must carry an AstFuncId; got {r:?}"
        );
        let distinct = r
            .metrics
            .get("distinct_types")
            .copied()
            .expect("distinct_types metric");
        assert!(distinct >= 1.0, "every fn touches at least one type");
    }
}

#[test]
fn rustdoc_levels_are_ok_or_skipped_with_a_reason() {
    let envelope = run_extract_on_fixture();
    for id in ["type.trait_impl", "type.coherence"] {
        let report = envelope.levels.get(id).expect("level present");
        match report.status {
            LevelStatus::Ok => {
                // When ok, the level emits at least its impl_count /
                // quadrant scalars (zero is fine; "populated" is the
                // contract, "non-zero" is fixture-dependent).
                let prefix = format!("{id}.");
                let any = envelope.vector.0.keys().any(|k| k.starts_with(&prefix));
                assert!(any, "ok {id} must emit at least one vector key");
            }
            LevelStatus::Skipped => {
                assert!(
                    report.reason.as_ref().is_some_and(|r| !r.is_empty()),
                    "skipped {id} must explain itself"
                );
                let prefix = format!("{id}.");
                let leaked = envelope.vector.0.keys().find(|k| k.starts_with(&prefix));
                assert!(
                    leaked.is_none(),
                    "skipped level {id} leaked key {leaked:?} (spec §8)"
                );
            }
            LevelStatus::Failed => panic!(
                "{id} failed on a well-formed fixture (reason: {:?})",
                report.reason
            ),
        }
    }
}

#[test]
fn with_impls_fixture_classifies_orphan_rule_quadrants() {
    // The `with-impls` fixture is the load-bearing scenario for level 19:
    //   - 2 local-trait + local-type impls (Describe / Reverse for Counter)
    //   - 1 foreign-trait + local-type impl  (Display for Counter)
    //   - 0 local-trait + foreign-type impls
    //   - 0 foreign-trait + foreign-type impls (no blankets that count)
    let envelope = run_extract_on_with_impls_fixture();
    let trait_impl = envelope.levels.get("type.trait_impl").expect("level present");
    let coherence = envelope.levels.get("type.coherence").expect("level present");

    // If nightly is unavailable both levels skip — only run the
    // substantive checks when rustdoc actually ran. Either outcome is
    // acceptable to spec §11.
    if !matches!(trait_impl.status, LevelStatus::Ok) {
        return;
    }
    assert_eq!(coherence.status, LevelStatus::Ok);

    // Level 17: at least the 2 local traits + 1 inherent + 3 user impls
    // are visible; rustdoc additionally synthesizes auto-trait + blanket
    // impls, so the totals will be larger than the source-visible count.
    let trait_count = envelope
        .vector
        .get("type.trait_impl.trait_count")
        .expect("trait_count populated");
    assert!(
        trait_count >= 2.0,
        "fixture defines two local traits; got {trait_count}"
    );
    let inherent = envelope
        .vector
        .get("type.trait_impl.inherent_impl_count")
        .expect("inherent_impl_count populated");
    assert!(
        inherent >= 1.0,
        "fixture defines an inherent impl on Counter; got {inherent}"
    );

    // Level 19: the user-written orphan-rule quadrants. We exclude
    // synthetic and blanket impls so these counts reflect the source.
    let llt = envelope
        .vector
        .get("type.coherence.local_trait_local_type")
        .expect("local_trait_local_type populated");
    let flt = envelope
        .vector
        .get("type.coherence.foreign_trait_local_type")
        .expect("foreign_trait_local_type populated");
    let lft = envelope
        .vector
        .get("type.coherence.local_trait_foreign_type")
        .expect("local_trait_foreign_type populated");
    let fft = envelope
        .vector
        .get("type.coherence.foreign_trait_foreign_type")
        .expect("foreign_trait_foreign_type populated");

    assert_eq!(llt, 2.0, "Describe + Reverse for Counter");
    assert_eq!(flt, 1.0, "Display for Counter");
    assert_eq!(lft, 0.0, "no local-trait + foreign-type impls in fixture");
    assert_eq!(fft, 0.0, "no foreign-trait + foreign-type impls in fixture");
}

// ───── Stage 9: graph-derived extractors ────────────────────────────────

/// The eight graph-derived level ids covered by Stage 9 (spec §2.7
/// 31, 32, 34, 36–40). Levels 33 (mono call graph) and 35 (data-flow)
/// land in Stage 14.
const STAGE9_LEVELS: &[&str] = &[
    "graph.mod",
    "graph.call_static",
    "graph.cfg",
    "graph.type_usage",
    "graph.sccs",
    "graph.communities",
    "graph.power_law",
    "graph.self_sim",
];

#[test]
fn stage_nine_graph_levels_report_ok_on_a_well_formed_fixture() {
    let envelope = run_extract_on_fixture();
    for id in STAGE9_LEVELS {
        let report = envelope
            .levels
            .get(*id)
            .unwrap_or_else(|| panic!("level {id} missing"));
        assert_eq!(
            report.status,
            LevelStatus::Ok,
            "stage-9 level {id} expected ok; got {:?} (reason: {:?})",
            report.status,
            report.reason
        );
        // Each ok level must contribute at least one vector key.
        let prefix = format!("{id}.");
        let any = envelope.vector.0.keys().any(|k| k.starts_with(&prefix));
        assert!(any, "ok level {id} emitted no vector keys");
    }
}

#[test]
fn fixture_call_and_mod_graphs_are_empty_with_no_uses_or_calls() {
    // The §13 fixture has a single file with two top-level fns that
    // never call each other. Spec §13 anchor: graph.call.edge_count = 0.
    let envelope = run_extract_on_fixture();
    assert_eq!(
        envelope.vector.get("graph.call_static.edge_count"),
        Some(0.0),
        "fixture's two fns don't call each other"
    );
    assert_eq!(
        envelope.vector.get("graph.call_static.node_count"),
        Some(2.0),
        "fixture has exactly two function nodes"
    );
    // Single-file crate → one module node, no edges.
    assert_eq!(
        envelope.vector.get("graph.mod.edge_count"),
        Some(0.0),
        "single-file crate has no use-clause edges"
    );
}

#[test]
fn with_graphs_fixture_observes_three_modules_and_call_edges() {
    let envelope = run_extract_on_with_graphs_fixture();

    // Module graph: three modules (`crate`, `crate::helpers`,
    // `crate::compute`) with two use-driven edges (helpers→compute and
    // compute→helpers).
    let mod_nodes = envelope
        .vector
        .get("graph.mod.node_count")
        .expect("graph.mod.node_count populated");
    assert_eq!(mod_nodes, 3.0, "fixture has three modules; got {mod_nodes}");

    let mod_edges = envelope
        .vector
        .get("graph.mod.edge_count")
        .expect("graph.mod.edge_count populated");
    assert!(
        mod_edges >= 2.0,
        "fixture's reciprocal `use` clauses must produce ≥ 2 edges; got {mod_edges}"
    );

    // Call graph: 5 functions, multiple calls. Edge count must be > 0
    // and node count must be 5.
    let call_nodes = envelope
        .vector
        .get("graph.call_static.node_count")
        .expect("graph.call_static.node_count populated");
    assert_eq!(call_nodes, 5.0, "fixture has 5 functions; got {call_nodes}");

    let call_edges = envelope
        .vector
        .get("graph.call_static.edge_count")
        .expect("graph.call_static.edge_count populated");
    assert!(call_edges >= 4.0, "fixture has multiple calls; got {call_edges}");
}

#[test]
fn with_graphs_fixture_per_function_cfg_records_carry_ast_func_ids() {
    let envelope = run_extract_on_with_graphs_fixture();
    let records = envelope
        .entities
        .get("graph.cfg_func")
        .expect("entities.graph.cfg_func populated");
    // Five functions: run, clamp, boosted, scale, quadratic.
    assert_eq!(records.len(), 5, "five fns → five CFG records");
    for r in records {
        assert!(
            r.ids.ast_func.is_some(),
            "every CFG record must carry an AstFuncId; got {r:?}"
        );
        let nodes = r
            .metrics
            .get("nodes")
            .copied()
            .expect("CFG record has node count");
        assert!(nodes >= 1.0, "every fn has at least one basic block; got {nodes}");
    }

    // `clamp` has two `if`/`else if` branches → branch_count ≥ 2.
    let clamp = records
        .iter()
        .find(|r| {
            r.ids
                .ast_func
                .as_ref()
                .is_some_and(|id| id.fq_path.ends_with("::clamp"))
        })
        .expect("clamp fn present");
    let branches = clamp
        .metrics
        .get("branches")
        .copied()
        .expect("clamp.branches");
    assert!(
        branches >= 2.0,
        "clamp has if/else-if/else; expected ≥ 2 branches, got {branches}"
    );
}

#[test]
fn fixture_type_usage_graph_populates_function_and_type_counts() {
    let envelope = run_extract_on_fixture();
    let funcs = envelope
        .vector
        .get("graph.type_usage.function_count")
        .expect("graph.type_usage.function_count populated");
    let types = envelope
        .vector
        .get("graph.type_usage.type_count")
        .expect("graph.type_usage.type_count populated");
    let edges = envelope
        .vector
        .get("graph.type_usage.edge_count")
        .expect("graph.type_usage.edge_count populated");
    assert_eq!(funcs, 2.0, "fixture has two functions");
    assert!(
        types >= 2.0,
        "fixture references at least i32 and str; got {types}"
    );
    assert!(
        edges >= 2.0,
        "fixture has at least 2 fn→type edges; got {edges}"
    );
}

#[test]
fn fixture_scc_and_community_metrics_are_well_formed() {
    let envelope = run_extract_on_with_graphs_fixture();
    // Every fn-node is its own SCC (call graph has cycles only via
    // recursion, which the fixture doesn't have). So SCC count == node count.
    let n = envelope
        .vector
        .get("graph.call_static.node_count")
        .expect("call_static node_count populated");
    let scc_count = envelope
        .vector
        .get("graph.sccs.call_static.count")
        .expect("graph.sccs.call_static.count populated");
    assert_eq!(scc_count, n, "no recursion → one SCC per fn");
    // No non-trivial SCCs (no cycles).
    assert_eq!(
        envelope
            .vector
            .get("graph.sccs.call_static.nontrivial_count"),
        Some(0.0),
        "fixture has no recursive calls"
    );

    // Community detection emits a modularity Q in [-0.5, 1.0]; check the
    // value is finite and in-range.
    let q = envelope
        .vector
        .get("graph.communities.call_static.modularity_q")
        .expect("modularity_q populated");
    assert!(q.is_finite(), "modularity Q must be finite; got {q}");
    assert!(
        (-0.5..=1.0).contains(&q),
        "modularity Q must lie in [-0.5, 1.0]; got {q}"
    );
}

#[test]
fn fixture_power_law_and_self_sim_emit_finite_scalars() {
    let envelope = run_extract_on_with_graphs_fixture();
    let alpha = envelope
        .vector
        .get("graph.power_law.call_static.degree_exponent")
        .expect("degree_exponent populated");
    assert!(alpha.is_finite(), "α must be finite; got {alpha}");
    assert!(alpha >= 0.0, "α must be non-negative; got {alpha}");

    let n = envelope
        .vector
        .get("graph.power_law.call_static.sample_count")
        .expect("sample_count populated");
    assert!(n >= 0.0, "sample_count must be non-negative; got {n}");

    for key in [
        "graph.self_sim.branch.func_vs_file.ks_distance",
        "graph.self_sim.branch.func_vs_module.ks_distance",
        "graph.self_sim.branch.file_vs_module.ks_distance",
    ] {
        let d = envelope.vector.get(key).expect("ks distance populated");
        assert!(d.is_finite(), "{key} must be finite; got {d}");
        assert!(
            (0.0..=1.0).contains(&d),
            "{key} must be a KS distance in [0,1]; got {d}"
        );
    }
}

#[test]
fn graph_level_vector_keys_use_their_level_namespace() {
    // Spec §5: vector keys are namespaced by their level's id. Anchor
    // test 5 (no two levels claim the same key) is already exercised
    // generally by the provenance test; this one specifically asserts
    // that every Stage-9 emission lives under its level's prefix.
    let envelope = run_extract_on_with_graphs_fixture();
    for (key, _) in envelope.vector.iter() {
        for id in STAGE9_LEVELS {
            // If a key is claimed by `id`, it must start with `id.`.
            let owner = envelope
                .provenance
                .0
                .get(key)
                .and_then(|v| v.first())
                .map(|s| s.as_str())
                .unwrap_or("");
            if owner == *id {
                let prefix = format!("{id}.");
                assert!(
                    key.starts_with(&prefix),
                    "key {key:?} owned by {id} should live under {prefix:?}"
                );
            }
        }
    }
}

// ───── Stage 10: external-tool extractors ──────────────────────────────

/// Spec §2.9 levels covered by Stage 10. Each may be `ok` or
/// `skipped`; on a well-formed fixture none should be `failed`.
const STAGE10_LEVELS: &[&str] = &["clippy", "rustdoc", "tests", "doctests"];

#[test]
fn stage_ten_levels_appear_with_a_documented_status() {
    let envelope = run_extract_on_fixture();
    for id in STAGE10_LEVELS {
        let report = envelope
            .levels
            .get(*id)
            .unwrap_or_else(|| panic!("level {id} missing from envelope"));
        match report.status {
            LevelStatus::Ok | LevelStatus::Skipped => {
                if report.status == LevelStatus::Skipped {
                    assert!(
                        report.reason.as_ref().is_some_and(|r| !r.is_empty()),
                        "skipped {id} must explain itself"
                    );
                }
            }
            LevelStatus::Failed => panic!(
                "stage-10 level {id} should not fail on a well-formed fixture (reason: {:?})",
                report.reason
            ),
        }
    }
}

#[test]
fn stage_ten_levels_obey_the_omission_contract() {
    // Spec §8: skipped/failed levels must not leak vector keys under
    // their level-id prefix. Ok levels must emit at least one key.
    let envelope = run_extract_on_fixture();
    for id in STAGE10_LEVELS {
        let report = envelope.levels.get(*id).expect("level present");
        let prefix = format!("{id}.");
        let any_key = envelope.vector.0.keys().any(|k| k.starts_with(&prefix));
        match report.status {
            LevelStatus::Ok => assert!(any_key, "ok level {id} emitted no keys"),
            LevelStatus::Skipped => {
                let leaked = envelope.vector.0.keys().find(|k| k.starts_with(&prefix));
                assert!(
                    leaked.is_none(),
                    "skipped level {id} leaked key {leaked:?} (spec §8)"
                );
            }
            LevelStatus::Failed => unreachable!("ruled out by previous test"),
        }
    }
}

#[test]
fn fixture_tests_and_doctests_inventories_show_zero_on_the_lib_fixture() {
    // The §13 fixture has no `#[test]` functions and no doctests. Spec
    // §8: ok levels with zero matter still must populate their summary
    // keys (so the consumer can distinguish "no tests" from "level
    // didn't run"). If the level is skipped on this host that's also
    // ok — bail early.
    let envelope = run_extract_on_fixture();

    let tests_report = envelope.levels.get("tests").expect("tests level present");
    if matches!(tests_report.status, LevelStatus::Ok) {
        let count = envelope
            .vector
            .get("tests.test_count")
            .expect("tests.test_count populated");
        assert_eq!(count, 0.0, "fixture defines no tests; got {count}");
        let binaries = envelope
            .vector
            .get("tests.binary_count")
            .expect("tests.binary_count populated");
        assert!(
            binaries >= 1.0,
            "even an empty lib produces one test binary; got {binaries}"
        );
    }

    let dt_report = envelope
        .levels
        .get("doctests")
        .expect("doctests level present");
    if matches!(dt_report.status, LevelStatus::Ok) {
        let count = envelope
            .vector
            .get("doctests.test_count")
            .expect("doctests.test_count populated");
        assert_eq!(count, 0.0, "fixture has no doctests; got {count}");
    }
}

#[test]
fn fixture_clippy_level_yields_finite_lint_counts() {
    // The §13 fixture is trivially clean — clippy may produce zero or a
    // handful of pedantic lints depending on the host's clippy version.
    // The contract: when ok, the summary scalars exist and are finite.
    let envelope = run_extract_on_fixture();
    let report = envelope.levels.get("clippy").expect("clippy level present");
    if !matches!(report.status, LevelStatus::Ok) {
        return;
    }
    for key in [
        "clippy.message_count",
        "clippy.lint_count",
        "clippy.warning_count",
        "clippy.error_count",
        "clippy.distinct_lint_names",
    ] {
        let v = envelope
            .vector
            .get(key)
            .unwrap_or_else(|| panic!("{key} missing"));
        assert!(v.is_finite() && v >= 0.0, "{key} must be a finite count; got {v}");
    }
    // The fixture compiles, so no errors.
    assert_eq!(
        envelope.vector.get("clippy.error_count"),
        Some(0.0),
        "fixture must compile cleanly under clippy"
    );
}

#[test]
fn fixture_rustdoc_inventory_reports_two_local_functions_when_available() {
    // Level 46 needs nightly to produce rustdoc JSON. When ok we expect
    // the two pub fns (add + classify) to show up as `function` kind
    // entries; when skipped the level explains itself.
    let envelope = run_extract_on_fixture();
    let report = envelope.levels.get("rustdoc").expect("rustdoc level present");
    match report.status {
        LevelStatus::Ok => {
            let fns = envelope
                .vector
                .get("rustdoc.kind.function")
                .expect("rustdoc.kind.function populated when level is ok");
            assert!(
                fns >= 2.0,
                "fixture has two pub functions; rustdoc should list at least that many, got {fns}"
            );
            let local = envelope
                .vector
                .get("rustdoc.local_item_count")
                .expect("rustdoc.local_item_count populated");
            assert!(local >= 2.0, "local_item_count must include the two fns");
        }
        LevelStatus::Skipped => {
            assert!(
                report.reason.as_ref().is_some_and(|r| !r.is_empty()),
                "skipped rustdoc level must explain itself"
            );
        }
        LevelStatus::Failed => panic!("rustdoc failed on a clean fixture: {:?}", report.reason),
    }
}

// ───── Stage 13: rustc-derived Level impls (nightly subprocess) ────────

/// The seven nightly-IR level ids covered by Stage 13 (spec §2.2 #10,
/// §2.3 #11–15, §2.4 #18).
const STAGE13_LEVELS: &[&str] = &[
    "ast.resolved",
    "hir",
    "thir",
    "mir.preopt",
    "mir.opt",
    "borrowck",
    "type.mono",
];

#[test]
fn stage_thirteen_levels_skip_cleanly_without_the_flag() {
    // Without --enable-rustc-internal the seven IR levels must all
    // report Skipped with a reason that tells the user how to enable
    // them. Spec §8 omission contract: no IR vector keys leak.
    let envelope = run_extract_on_fixture();
    for id in STAGE13_LEVELS {
        let report = envelope.levels.get(*id).expect("level present");
        assert_eq!(
            report.status,
            LevelStatus::Skipped,
            "{id} should skip when rustc-internal extraction is disabled; got {:?}",
            report.status
        );
        let reason = report.reason.as_deref().unwrap_or("");
        assert!(
            !reason.is_empty(),
            "{id} skipped without a reason — user has no way to know how to enable it"
        );
    }
    for key in envelope.vector.0.keys() {
        for prefix in [
            "ast.resolved.",
            "hir.",
            "thir.",
            "mir.preopt.",
            "mir.opt.",
            "borrowck.",
            "type.mono.",
        ] {
            assert!(
                !key.starts_with(prefix),
                "skipped rustc-internal level emitted {key} (spec §8 says skipped levels omit their keys)"
            );
        }
    }
}

/// Shared --enable-rustc-internal extract on the fixture. Cached so
/// the several scenario tests below don't each pay the cost of
/// building and running the nightly sub-binary.
fn run_extract_on_fixture_with_rustc_internal() -> &'static OracleOutput {
    static CACHE: OnceLock<OracleOutput> = OnceLock::new();
    CACHE.get_or_init(|| {
        let fixture = workspace_root().join("examples").join("single-file-lib");
        assert!(fixture.is_dir(), "fixture missing: {fixture:?}");
        run_extract_with(&fixture, &["--enable-rustc-internal"])
    })
}

#[test]
fn stage_thirteen_levels_appear_with_a_documented_status_when_enabled() {
    // With --enable-rustc-internal each level is either ok (nightly
    // present and the extractor ran) or skipped (nightly absent, or
    // the sub-binary couldn't be located/built on this host). Failed
    // is acceptable too — the spec §11 contract is "documented
    // status with non-empty reason", not "guaranteed ok".
    let envelope = run_extract_on_fixture_with_rustc_internal();
    for id in STAGE13_LEVELS {
        let report = envelope.levels.get(*id).expect("level present");
        match report.status {
            LevelStatus::Ok => {
                let prefix = format!("{id}.");
                let any = envelope.vector.0.keys().any(|k| k.starts_with(&prefix));
                assert!(any, "ok {id} must emit at least one vector key");
            }
            LevelStatus::Skipped | LevelStatus::Failed => {
                assert!(
                    report.reason.as_ref().is_some_and(|r| !r.is_empty()),
                    "{id} non-ok must explain itself"
                );
                let prefix = format!("{id}.");
                let leaked = envelope.vector.0.keys().find(|k| k.starts_with(&prefix));
                assert!(
                    leaked.is_none(),
                    "{id} {:?} leaked key {leaked:?} (spec §8)",
                    report.status
                );
            }
        }
    }
}

#[test]
fn stage_thirteen_levels_share_one_reason_when_nightly_missing() {
    // If nightly isn't available, all seven IR levels must report the
    // same reason — the single subprocess invocation skipped once and
    // every level routed the same diagnostic back. This stops the
    // user from seeing seven different complaints when there is only
    // one underlying cause.
    let envelope = run_extract_on_fixture_with_rustc_internal();
    let reasons: BTreeSet<String> = STAGE13_LEVELS
        .iter()
        .map(|id| {
            envelope
                .levels
                .get(*id)
                .and_then(|r| r.reason.clone())
                .unwrap_or_default()
        })
        .collect();
    let any_ok = STAGE13_LEVELS.iter().any(|id| {
        envelope
            .levels
            .get(*id)
            .map(|r| r.status == LevelStatus::Ok)
            .unwrap_or(false)
    });
    if !any_ok {
        assert_eq!(
            reasons.len(),
            1,
            "every IR level should share one reason when the cache fails once; got {reasons:?}"
        );
    }
}

#[test]
fn stage_thirteen_fixture_reflects_two_functions_when_ok() {
    // When nightly is available and the sub-binary runs, the fixture
    // (two pub fns: add + classify) must produce observable signals:
    // ≥ 2 HIR fns, ≥ 2 THIR bodies, ≥ 2 MIR bodies (pre and opt), and
    // ≥ 2 mono items. Exact values are rustc-version-sensitive; we
    // lower-bound them.
    let envelope = run_extract_on_fixture_with_rustc_internal();
    let hir_status = &envelope.levels.get("hir").expect("hir present").status;
    if !matches!(hir_status, LevelStatus::Ok) {
        // Nightly unavailable or extractor couldn't run; the previous
        // tests already cover that case.
        return;
    }

    let hir_fns = envelope
        .vector
        .get("hir.fn_count")
        .expect("hir.fn_count populated when ok");
    assert!(
        hir_fns >= 2.0,
        "fixture has two pub fns; hir.fn_count must be ≥ 2, got {hir_fns}"
    );

    let thir_bodies = envelope
        .vector
        .get("thir.body_count")
        .expect("thir.body_count populated when ok");
    assert!(
        thir_bodies >= 2.0,
        "thir.body_count must reflect both fixture fns, got {thir_bodies}"
    );

    for level_id in ["mir.preopt", "mir.opt"] {
        let key = format!("{level_id}.body_count");
        let n = envelope
            .vector
            .get(&key)
            .unwrap_or_else(|| panic!("{key} populated when ok"));
        assert!(n >= 2.0, "{key} must be ≥ 2, got {n}");

        // classify has two if-expressions → ≥ 4 basic blocks in MIR.
        let bb_max_key = format!("{level_id}.basic_block_count.max");
        let bb_max = envelope
            .vector
            .get(&bb_max_key)
            .unwrap_or_else(|| panic!("{bb_max_key} populated when ok"));
        assert!(
            bb_max >= 4.0,
            "{bb_max_key} must capture classify's branching (≥ 4); got {bb_max}"
        );
    }

    let mono_items = envelope
        .vector
        .get("type.mono.item_count")
        .expect("type.mono.item_count populated when ok");
    assert!(
        mono_items >= 2.0,
        "fixture's two non-generic fns must produce ≥ 2 mono items, got {mono_items}"
    );
    // Non-generic fns yield exactly one instantiation each → no group
    // has more than one instance.
    let multi = envelope
        .vector
        .get("type.mono.multi_instance_group_count")
        .expect("type.mono.multi_instance_group_count populated when ok");
    assert_eq!(
        multi, 0.0,
        "fixture has no generics; multi-instance groups must be 0"
    );

    // HIR expr-kind histogram must include "if" with ≥ 2 (classify has
    // two if-expressions).
    let if_kinds = envelope
        .vector
        .get("hir.expr_kind.if")
        .unwrap_or(0.0);
    assert!(
        if_kinds >= 2.0,
        "hir.expr_kind.if should reflect classify's two ifs, got {if_kinds}"
    );

    // borrowck and ast.resolved levels emit at least their core scalars.
    let region_sum = envelope
        .vector
        .get("borrowck.region_count.sum")
        .expect("borrowck.region_count.sum populated when ok");
    assert!(
        region_sum.is_finite() && region_sum >= 0.0,
        "borrowck.region_count.sum must be finite non-negative, got {region_sum}"
    );
    let imports = envelope
        .vector
        .get("ast.resolved.import_count")
        .expect("ast.resolved.import_count populated when ok");
    assert!(
        imports.is_finite() && imports >= 0.0,
        "ast.resolved.import_count must be finite non-negative, got {imports}"
    );
}

// ───── Stage 14: derived cross-level extractors (33, 35) ────────────────

/// The two Stage-14 derived levels (spec §2.7 #33 + #35).
const STAGE14_LEVELS: &[&str] = &["graph.call_mono", "graph.dataflow"];

#[test]
fn stage_fourteen_levels_skip_cleanly_without_codegen() {
    // The mono-call-graph and per-function dataflow levels both compose
    // codegen-side artifacts. Without `--enable-codegen` they must skip
    // with a documented reason and contribute no vector keys (spec §8).
    let envelope = run_extract_on_fixture();
    for id in STAGE14_LEVELS {
        let report = envelope.levels.get(*id).expect("level present");
        assert_eq!(
            report.status,
            LevelStatus::Skipped,
            "{id} should skip without codegen; got {:?}",
            report.status
        );
        let reason = report.reason.as_deref().unwrap_or("");
        assert!(!reason.is_empty(), "{id} skipped without a reason");
    }
    for key in envelope.vector.0.keys() {
        assert!(
            !key.starts_with("graph.call_mono.") && !key.starts_with("graph.dataflow."),
            "skipped Stage-14 level emitted {key} (spec §8 says skipped levels omit their keys)"
        );
    }
}

#[test]
fn stage_fourteen_levels_report_ok_with_codegen_on_the_fixture() {
    let envelope = run_extract_on_fixture_with_codegen();
    for id in STAGE14_LEVELS {
        let report = envelope.levels.get(*id).expect("level present");
        assert_eq!(
            report.status,
            LevelStatus::Ok,
            "{id} expected ok with codegen; got {:?} (reason: {:?})",
            report.status,
            report.reason
        );
    }
}

#[test]
fn fixture_call_mono_graph_observes_at_least_the_two_pub_functions() {
    // Spec §7 Stage 14 done-when: mono-call-graph edges and per-function
    // dataflow density scalars appear in the vector for the fixture.
    // The fixture has two top-level pub fns; the codegen build emits
    // each as its own LLVM function, so the mono node count is ≥ 2 and
    // every node carries both a SymbolId and a MonoId.
    let envelope = run_extract_on_fixture_with_codegen();
    let node_count = envelope
        .vector
        .get("graph.call_mono.node_count")
        .expect("graph.call_mono.node_count populated");
    assert!(
        node_count >= 2.0,
        "two pub fns must produce ≥ 2 mono nodes, got {node_count}"
    );

    // The fixture has no cross-fn calls, so the in-graph edge count is
    // 0 and there are exactly as many leaves as nodes (every fn is a
    // leaf in its own call graph). External calls (to panic helpers,
    // intrinsics, etc.) may still register on the external side.
    let edge_count = envelope
        .vector
        .get("graph.call_mono.edge_count")
        .expect("graph.call_mono.edge_count populated");
    assert!(
        edge_count.is_finite() && edge_count >= 0.0,
        "edge_count must be finite ≥ 0, got {edge_count}"
    );
    let leaf_count = envelope
        .vector
        .get("graph.call_mono.leaf_count")
        .expect("graph.call_mono.leaf_count populated");
    assert!(leaf_count >= 2.0, "≥ 2 leaves expected, got {leaf_count}");

    let recursive = envelope
        .vector
        .get("graph.call_mono.recursive_node_count")
        .expect("graph.call_mono.recursive_node_count populated");
    assert_eq!(recursive, 0.0, "fixture has no recursion; got {recursive}");

    // Degree summaries must be finite even on graphs with zero edges.
    for key in [
        "graph.call_mono.in_degree.max",
        "graph.call_mono.in_degree.mean",
        "graph.call_mono.out_degree.max",
        "graph.call_mono.out_degree.mean",
        "graph.call_mono.density",
    ] {
        let v = envelope
            .vector
            .get(key)
            .unwrap_or_else(|| panic!("{key} missing"));
        assert!(v.is_finite() && v >= 0.0, "{key} must be finite ≥ 0; got {v}");
    }

    // Per-node entities: each carries SymbolId + MonoId per spec §6.
    let records = envelope
        .entities
        .get("graph.call_mono_node")
        .expect("entities.graph.call_mono_node populated when ok");
    assert!(
        records.len() >= 2,
        "expected ≥ 2 mono-node records, got {}",
        records.len()
    );
    for r in records {
        assert!(
            r.ids.symbol.is_some(),
            "every mono-node record must carry a SymbolId; got {r:?}"
        );
        assert!(
            r.ids.mono.is_some(),
            "every mono-node record must carry a MonoId; got {r:?}"
        );
    }
}

#[test]
fn fixture_dataflow_density_is_a_finite_per_function_scalar() {
    let envelope = run_extract_on_fixture_with_codegen();

    let n = envelope
        .vector
        .get("graph.dataflow.function_count")
        .expect("graph.dataflow.function_count populated");
    assert!(n >= 2.0, "at least two functions expected, got {n}");

    // Aggregate scalars must be finite and well-typed. density.mean is
    // dimensionless (edges per instruction) but, in pathological cases
    // with heavy reuse, can exceed 1 — only the finite+non-negative
    // contract is load-bearing.
    for key in [
        "graph.dataflow.def_count.sum",
        "graph.dataflow.def_count.mean",
        "graph.dataflow.def_count.max",
        "graph.dataflow.use_count.sum",
        "graph.dataflow.use_count.mean",
        "graph.dataflow.use_count.max",
        "graph.dataflow.edge_count.sum",
        "graph.dataflow.edge_count.mean",
        "graph.dataflow.edge_count.max",
        "graph.dataflow.density.mean",
        "graph.dataflow.density.max",
    ] {
        let v = envelope
            .vector
            .get(key)
            .unwrap_or_else(|| panic!("{key} missing"));
        assert!(v.is_finite() && v >= 0.0, "{key} must be finite ≥ 0; got {v}");
    }

    // At least one function in this fixture (classify) is non-trivial
    // and must have a non-zero def-use edge count.
    let edge_max = envelope
        .vector
        .get("graph.dataflow.edge_count.max")
        .unwrap();
    assert!(
        edge_max > 0.0,
        "fixture's classify body must have some def-use chain, got max={edge_max}"
    );

    // Per-function entity records: density present, finite, ≥ 0.
    let records = envelope
        .entities
        .get("graph.dataflow_func")
        .expect("entities.graph.dataflow_func populated when ok");
    assert!(
        records.len() >= 2,
        "expected ≥ 2 dataflow records, got {}",
        records.len()
    );
    for r in records {
        let density = r
            .metrics
            .get("density")
            .copied()
            .unwrap_or_else(|| panic!("per-fn density missing: {r:?}"));
        assert!(
            density.is_finite() && density >= 0.0,
            "per-fn density must be finite ≥ 0; got {density}"
        );
        assert!(
            r.ids.symbol.is_some(),
            "every dataflow record must carry a SymbolId; got {r:?}"
        );
    }
}

#[test]
fn stage_fourteen_keys_obey_their_level_namespace() {
    // Spec §4 + §7: each level owns its key prefix; provenance must
    // attribute each Stage-14 key to exactly its owning level.
    let envelope = run_extract_on_fixture_with_codegen();
    for (key, owners) in &envelope.provenance.0 {
        if key.starts_with("graph.call_mono.") {
            assert_eq!(
                owners,
                &vec!["graph.call_mono".to_string()],
                "{key} should be owned only by graph.call_mono; got {owners:?}"
            );
        }
        if key.starts_with("graph.dataflow.") {
            assert_eq!(
                owners,
                &vec!["graph.dataflow".to_string()],
                "{key} should be owned only by graph.dataflow; got {owners:?}"
            );
        }
    }
}
