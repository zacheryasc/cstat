//! The eight anchor tests from `milestone-1-dev-plan.md` §"Anchor tests".
//!
//! Each test corresponds 1-to-1 with a numbered anchor from the dev plan.
//! They are kept in this dedicated file so that a stranger reviewing the
//! repository can see the contract Milestone 1 satisfies in one place,
//! independent of the per-stage scenario tests in `extract_fixture.rs`.
//!
//! These tests observe the §7 envelope only — they survive any refactor
//! that preserves the JSON contract. Internal helpers (functions,
//! `Level` impls, intermediate types) are not referenced.
//!
//! Conventions:
//! - Tests are named `anchor_<n>_*` to match the numbering in the dev plan.
//! - Anchor 8 is a structural CI invariant — implemented here as a Rust
//!   test that shells out to `cargo tree`, which keeps the contract
//!   enforceable from `cargo test` without a separate CI script.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use cstat_orchestrate::REQUIRED_LEVELS;
use cstat_schema::{LevelStatus, OracleOutput};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root resolves")
}

fn run_extract(project: &Path, extra_args: &[&str]) -> OracleOutput {
    run_extract_with_env(project, extra_args, &[])
}

fn run_extract_with_env(
    project: &Path,
    extra_args: &[&str],
    extra_env: &[(&str, &str)],
) -> OracleOutput {
    let bin = env!("CARGO_BIN_EXE_cstat");
    let mut cmd = Command::new(bin);
    cmd.args(["extract", "--path"]).arg(project);
    cmd.args(extra_args);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("invoke cstat binary");
    assert!(
        output.status.success(),
        "cstat extract on {project:?} failed: status={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout parses as OracleOutput envelope")
}

fn fixture_path(name: &str) -> PathBuf {
    let p = workspace_root().join("examples").join(name);
    assert!(p.is_dir(), "fixture {name} missing at {p:?}");
    p
}

/// Cached single-file-lib extract under the default config. Many anchor
/// tests share this baseline.
fn default_envelope() -> &'static OracleOutput {
    static CACHE: OnceLock<OracleOutput> = OnceLock::new();
    CACHE.get_or_init(|| run_extract(&fixture_path("single-file-lib"), &[]))
}

/// Strip every field that the spec licenses to drift between two runs
/// of the same project on the same host. Used by anchor 3 (determinism)
/// to compare runs without losing to wall-clock jitter.
///
/// What we drop:
/// - `wall_clock_seconds` — total elapsed.
/// - `extraction_seconds` on every level report — per-level elapsed.
/// - The `vector` is filtered to the subset whose owning level reports
///   `deterministic: true`. Non-deterministic keys are checked
///   separately by the test (they're allowed to drift).
fn projection_for_determinism(env: &OracleOutput) -> serde_json::Value {
    let mut clone = env.clone();
    clone.wall_clock_seconds = 0.0;
    for report in clone.levels.values_mut() {
        report.extraction_seconds = 0.0;
    }

    // Drop vector keys whose owning level is non-deterministic.
    let nondet_levels: BTreeSet<String> = clone
        .levels
        .iter()
        .filter(|(_, r)| !r.deterministic)
        .map(|(id, _)| id.clone())
        .collect();
    let mut keys_to_drop = Vec::new();
    for (key, owners) in clone.provenance.0.iter() {
        if owners.iter().any(|o| nondet_levels.contains(o)) {
            keys_to_drop.push(key.clone());
        }
    }
    for key in &keys_to_drop {
        clone.vector.0.remove(key);
        clone.provenance.0.remove(key);
    }

    // Entities aren't covered by the deterministic flag directly, but
    // every entity-kind in the envelope is namespaced by its owning
    // level; drop the ones owned by non-deterministic levels.
    let mut entity_keys_to_drop = Vec::new();
    for kind in clone.entities.keys() {
        let owner = kind.split('.').next().unwrap_or("");
        if nondet_levels.iter().any(|l| {
            l.split('.').next().unwrap_or("") == owner || l.as_str() == owner
        }) {
            entity_keys_to_drop.push(kind.clone());
        }
    }
    for kind in entity_keys_to_drop {
        clone.entities.remove(&kind);
    }

    serde_json::to_value(&clone).expect("envelope serialises")
}

// ───── Anchor 1: §13 worked example reproduces ─────────────────────────

/// The §13 fixture (`examples/single-file-lib/`) is a two-function lib
/// (`add`, `classify`). The spec illustrates several invariants on it:
/// two pub functions, AST↔Symbol identity join on `classify`,
/// `source_to_symbol_ratio == 1.0` (no generics), and the call/module
/// graphs being trivially small.
///
/// This anchor checks the *shape* of those invariants against the live
/// envelope. Numeric values from §13 that are host-sensitive (clippy
/// lint counts, exact section sizes) are deliberately not asserted —
/// the spec says those numbers were illustrative and may need
/// correction.
#[test]
fn anchor_1_worked_example_reproduces() {
    let env = default_envelope();

    // §13 anchor: two top-level pub fns observable at the AST level.
    assert_eq!(
        env.vector.get("ast.global.func_count"),
        Some(2.0),
        "§13: fixture has exactly two functions"
    );
    assert_eq!(
        env.vector.get("ast.cst.parsed_file_count"),
        Some(1.0),
        "§13: single-file fixture parses one .rs file"
    );

    // §13 anchor: the trivially-empty intra-crate call graph
    // (the two fns don't call each other).
    assert_eq!(
        env.vector.get("graph.call_static.node_count"),
        Some(2.0),
        "§13: two call-graph nodes"
    );
    assert_eq!(
        env.vector.get("graph.call_static.edge_count"),
        Some(0.0),
        "§13: zero call-graph edges (fns are independent)"
    );

    // §13 anchor: schema and toolchain stamp populated.
    assert!(!env.schema_version.is_empty());
    assert!(!env.cstat_version.is_empty());
    assert!(!env.toolchain.channel.is_empty());
    assert!(!env.toolchain.version.is_empty());

    // §13 anchor: every required level is in the levels map (the
    // strong form of this is anchor 2; here we just check the count).
    assert_eq!(env.levels.len(), REQUIRED_LEVELS.len());
}

// ───── Anchor 2: every required level is present ───────────────────────

/// For any non-empty Rust project, every level in `REQUIRED_LEVELS`
/// appears in the envelope's `levels` map with a status in
/// `{ok, skipped, failed}`. Non-ok statuses must carry a non-empty
/// `reason`. Spec §4 / dev-plan anchor 2.
#[test]
fn anchor_2_every_required_level_is_present() {
    let env = default_envelope();
    for descriptor in REQUIRED_LEVELS {
        let report = env
            .levels
            .get(descriptor.id)
            .unwrap_or_else(|| panic!("required level {} missing from envelope", descriptor.id));
        match report.status {
            LevelStatus::Ok => {}
            LevelStatus::Skipped | LevelStatus::Failed => {
                let reason = report.reason.as_deref().unwrap_or("");
                assert!(
                    !reason.is_empty(),
                    "{} reports {:?} without a reason — spec §11 violation",
                    descriptor.id,
                    report.status
                );
            }
        }
    }
}

// ───── Anchor 3: determinism on deterministic-tagged keys ──────────────

/// Two consecutive extracts of the same project must produce
/// byte-identical JSON for every key whose owning level reports
/// `deterministic: true`. Non-deterministic keys are inspected
/// separately. Dev-plan anchor 3.
#[test]
fn anchor_3_deterministic_keys_byte_match_across_runs() {
    let fixture = fixture_path("single-file-lib");
    let first = run_extract(&fixture, &[]);
    let second = run_extract(&fixture, &[]);

    let first_proj = projection_for_determinism(&first);
    let second_proj = projection_for_determinism(&second);

    if first_proj != second_proj {
        let mut diffs = Vec::new();
        diff_value("", &first_proj, &second_proj, &mut diffs);
        panic!(
            "deterministic projection differs between runs:\n{}",
            diffs.join("\n")
        );
    }
}

/// Walk two JSON values in parallel and emit a textual list of every
/// path where they disagree. Used only by the determinism anchor's
/// failure path.
fn diff_value(path: &str, a: &serde_json::Value, b: &serde_json::Value, out: &mut Vec<String>) {
    use serde_json::Value;
    match (a, b) {
        (Value::Object(ao), Value::Object(bo)) => {
            for (k, va) in ao {
                let sub = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                match bo.get(k) {
                    Some(vb) => diff_value(&sub, va, vb, out),
                    None => out.push(format!("  {sub}: missing in second run (first={va})")),
                }
            }
            for k in bo.keys() {
                if !ao.contains_key(k) {
                    let sub = if path.is_empty() {
                        k.clone()
                    } else {
                        format!("{path}.{k}")
                    };
                    out.push(format!("  {sub}: missing in first run (second={})", bo[k]));
                }
            }
        }
        (Value::Array(aa), Value::Array(bb)) => {
            if aa.len() != bb.len() {
                out.push(format!(
                    "  {path}: array length differs ({} vs {})",
                    aa.len(),
                    bb.len()
                ));
            }
            for (i, (va, vb)) in aa.iter().zip(bb.iter()).enumerate() {
                diff_value(&format!("{path}[{i}]"), va, vb, out);
            }
        }
        _ if a != b => out.push(format!("  {path}: first={a} second={b}")),
        _ => {}
    }
}

// ───── Anchor 4: schema roundtrip ──────────────────────────────────────

/// `OracleOutput → JSON → serde_json::Value → JSON → OracleOutput` is
/// lossless for any output the runner can actually produce. Catches
/// accidental schema drift the moment it appears. Dev-plan anchor 4.
#[test]
fn anchor_4_envelope_roundtrips_losslessly_through_value() {
    let env = default_envelope();
    let original_json = serde_json::to_string(env).expect("envelope serialises");
    let value: serde_json::Value =
        serde_json::from_str(&original_json).expect("envelope JSON parses as Value");
    let reserialised = serde_json::to_string(&value).expect("Value reserialises");
    let decoded: OracleOutput =
        serde_json::from_str(&reserialised).expect("reserialised JSON parses as OracleOutput");
    assert_eq!(
        &decoded, env,
        "envelope did not round-trip through serde_json::Value"
    );
}

// ───── Anchor 5: no two levels claim the same key ──────────────────────

/// Across every registered level, the set of vector keys is pairwise
/// disjoint between levels. Provenance for each key names exactly one
/// owning level. The orchestrator already rejects double-claims at run
/// time (`OrchestrateError::NamespaceCollision`); this test makes the
/// invariant observable in the envelope shape. Dev-plan anchor 5.
#[test]
fn anchor_5_no_two_levels_claim_the_same_vector_key() {
    let env = default_envelope();
    let known: BTreeSet<&str> = REQUIRED_LEVELS
        .iter()
        .map(|d| d.id)
        .chain(std::iter::once("meta"))
        .collect();
    for (key, _) in env.vector.iter() {
        let owners = env
            .provenance
            .0
            .get(key)
            .unwrap_or_else(|| panic!("vector key {key:?} has no provenance entry"));
        assert_eq!(
            owners.len(),
            1,
            "vector key {key:?} has multiple owners {owners:?}"
        );
        let owner = owners[0].as_str();
        assert!(
            known.contains(owner),
            "vector key {key:?} attributed to unknown level {owner:?}"
        );
    }
}

// ───── Anchor 6: unparseable file contract (§11 row 4) ─────────────────

/// A project with one well-formed `.rs` file and one syntactically
/// broken one must produce `entities.ast.file_errors` with the bad
/// file, populated metrics for the good file, an exit code of zero,
/// and `meta.files_unparseable.count` reflecting the failure count.
/// Dev-plan anchor 6.
#[test]
fn anchor_6_unparseable_file_degrades_gracefully() {
    let env = run_extract(&fixture_path("broken-file"), &[]);

    // Exit zero is implicit — `run_extract` panics otherwise.

    // AST levels still report ok despite one file failing to parse.
    for id in ["ast.cst", "ast.items"] {
        let report = env.levels.get(id).expect("ast level present");
        assert_eq!(
            report.status,
            LevelStatus::Ok,
            "{id} must stay ok with one bad file (reason: {:?})",
            report.reason
        );
    }

    // The broken file is recorded under entities.ast.file_errors.
    let errors = env
        .entities
        .get("ast.file_errors")
        .expect("entities.ast.file_errors populated");
    assert_eq!(errors.len(), 1, "exactly one unparseable file expected");
    let file_id = errors[0]
        .ids
        .file
        .as_ref()
        .expect("file_errors entry carries a FileId");
    assert!(
        file_id.as_str().ends_with("bad.rs"),
        "file_errors should identify bad.rs, got {file_id:?}"
    );

    // meta counter agrees with the entity count.
    assert_eq!(
        env.vector.get("meta.files_unparseable.count"),
        Some(1.0),
        "meta.files_unparseable.count must mirror entities.ast.file_errors"
    );

    // The good file's metrics still land.
    assert_eq!(
        env.vector.get("ast.global.func_count"),
        Some(1.0),
        "good.rs's single fn still counted"
    );
}

// ───── Anchor 7: nightly unavailable contract (§11 "tool missing") ─────

/// Stable-only Rust levels — levels not gated on nightly, rustdoc JSON,
/// cargo-expand, or codegen. These must report `ok` on any well-formed
/// project, regardless of whether nightly is installed.
const STABLE_ONLY_LEVELS: &[&str] = &[
    "fs.layout",
    "fs.bytes",
    "fs.source",
    "tok.stream",
    "tok.comments",
    "fmt.whitespace",
    "ast.cst",
    "ast.items",
    "type.usage",
    "graph.mod",
    "graph.call_static",
    "graph.cfg",
    "graph.type_usage",
    "graph.sccs",
    "graph.communities",
    "graph.power_law",
    "graph.self_sim",
    "cargo.workspace",
    "cargo.lock",
    "cargo.cfg",
    "cargo.toolchain",
    "rustfmt",
];

/// Levels gated on a nightly toolchain (or a nightly-only subcommand
/// like rustdoc-JSON / cargo-expand). With the default `cstat extract`
/// these must each report a non-ok status with a non-empty reason —
/// either "nightly toolchain unavailable" (nightly absent) or the
/// equivalent "not enabled" reason for the opt-in flag.
const NIGHTLY_GATED_LEVELS: &[&str] = &[
    "ast.expanded",     // cargo-expand
    "ast.resolved",     // rustc-internal
    "hir",              // rustc-internal
    "thir",             // rustc-internal
    "mir.preopt",       // rustc-internal
    "mir.opt",          // rustc-internal
    "borrowck",         // rustc-internal
    "type.trait_impl",  // rustdoc-JSON
    "type.coherence",   // rustdoc-JSON
    "type.mono",        // rustc-internal
    "rustdoc",          // rustdoc-JSON
];

/// Default-config extract on a clean fixture, with the
/// `CSTAT_FORCE_NO_NIGHTLY=1` env override that simulates a host without
/// a nightly toolchain (dev plan §"Anchor tests" #7: "Fixture: any
/// project, run with an env override that simulates missing nightly").
///
/// Asserts: every stable-only level reports `ok`; every nightly-gated
/// level reports `{skipped, failed}` with a non-empty reason; spec §8
/// omission contract holds for the non-ok set; the process exits zero.
#[test]
fn anchor_7_nightly_unavailable_contract() {
    let env = run_extract_with_env(
        &fixture_path("single-file-lib"),
        &[],
        &[("CSTAT_FORCE_NO_NIGHTLY", "1")],
    );

    for id in STABLE_ONLY_LEVELS {
        let report = env
            .levels
            .get(*id)
            .unwrap_or_else(|| panic!("stable-only level {id} missing"));
        assert_eq!(
            report.status,
            LevelStatus::Ok,
            "stable-only level {id} should be ok regardless of nightly availability \
             (got {:?}, reason: {:?})",
            report.status,
            report.reason
        );
    }

    for id in NIGHTLY_GATED_LEVELS {
        let report = env
            .levels
            .get(*id)
            .unwrap_or_else(|| panic!("nightly-gated level {id} missing"));
        assert!(
            matches!(report.status, LevelStatus::Skipped | LevelStatus::Failed),
            "{id} should be skipped or failed under CSTAT_FORCE_NO_NIGHTLY (got {:?})",
            report.status
        );
        let reason = report.reason.as_deref().unwrap_or("");
        assert!(
            !reason.is_empty(),
            "{id} non-ok status without a reason — spec §11 violation"
        );

        // Spec §8 omission contract: non-ok levels must not emit keys
        // under their level-id prefix.
        let prefix = format!("{id}.");
        let leaked = env.vector.0.keys().find(|k| k.starts_with(&prefix));
        assert!(
            leaked.is_none(),
            "non-ok level {id} leaked vector key {leaked:?}"
        );
    }
}

// ───── Anchor 8: isolation boundary ────────────────────────────────────

/// `cargo tree` against every stable crate must not contain any
/// `rustc_*` crate in its transitive dependency graph. The single
/// crate permitted to link rustc internals is `cstat-extract-rustc`,
/// which is excluded from the default workspace. Dev-plan anchor 8.
#[test]
fn anchor_8_isolation_boundary_no_rustc_internals_in_stable_crates() {
    // Each of these must build on stable and must not pull any
    // `rustc_*` rustc-private crate into its tree.
    let stable_crates = [
        "cstat-schema",
        "cstat-ir",
        "cstat-orchestrate",
        "cstat-extract-stable",
        "cstat-cli",
    ];
    let root = workspace_root();
    for crate_name in stable_crates {
        let output = Command::new("cargo")
            .args(["tree", "-p", crate_name, "--prefix", "none", "--no-dedupe"])
            .current_dir(&root)
            .output()
            .unwrap_or_else(|e| panic!("invoke cargo tree -p {crate_name}: {e}"));
        assert!(
            output.status.success(),
            "cargo tree -p {crate_name} failed: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            // Lines are of the form "<crate> v<version>"; the crate
            // name comes first. We only want to fail on direct crate-
            // name matches (not, say, a path mention in a comment).
            let crate_token = line.split_whitespace().next().unwrap_or("");
            assert!(
                !crate_token.starts_with("rustc_"),
                "isolation boundary breached: {crate_name} depends on {crate_token} \
                 (full line: {line:?})"
            );
        }
    }
}
