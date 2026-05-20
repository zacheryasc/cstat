//! Level trait, runner, and output assembly (spec §7, §12).
//!
//! The runner executes each registered [`Level`], stamps its result into the
//! [`OracleOutput`] envelope, and auto-emits a `skipped` / `"not implemented"`
//! report for every required level that has no implementation registered yet.
//! This is the mechanism that keeps the §7 contract honest as later stages
//! land — a missing extractor is visible in the output, not silently absent
//! (spec §4, anchor test 2).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub use cstat_schema::{
    CostClass, EntityRecord, LevelReport, LevelStatus, OracleOutput, Provenance, Toolchain,
    Vector,
};

/// Static description of a representation level required by spec §4.
///
/// `id` is the stable short identifier used both in the §7 `levels` map and
/// as the prefix for the level's keys in the flat `vector` namespace.
/// `spec_number` is the numeric label from spec §2.1–§2.9; carried purely for
/// human cross-reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelDescriptor {
    pub id: &'static str,
    pub spec_number: u32,
    pub cost_class: CostClass,
    pub deterministic: bool,
}

/// The canonical list of representation levels the output envelope must
/// always cover (spec §4: §2.1–§2.9, excluding §2.10).
///
/// Order is the spec's order so the §7 `levels` map walks tidily when
/// presented to a human. The map itself is a `BTreeMap`, so the wire-format
/// iteration order is alphabetical by `id`.
pub const REQUIRED_LEVELS: &[LevelDescriptor] = &[
    // §2.1 Textual and pre-syntactic
    LevelDescriptor { id: "fs.layout",          spec_number: 1,  cost_class: CostClass::F, deterministic: true  },
    LevelDescriptor { id: "fs.bytes",           spec_number: 2,  cost_class: CostClass::F, deterministic: true  },
    LevelDescriptor { id: "fs.source",          spec_number: 3,  cost_class: CostClass::F, deterministic: true  },
    LevelDescriptor { id: "tok.stream",         spec_number: 4,  cost_class: CostClass::F, deterministic: true  },
    LevelDescriptor { id: "tok.comments",       spec_number: 5,  cost_class: CostClass::F, deterministic: true  },
    LevelDescriptor { id: "fmt.whitespace",     spec_number: 6,  cost_class: CostClass::F, deterministic: true  },
    // §2.2 Syntactic
    LevelDescriptor { id: "ast.cst",            spec_number: 7,  cost_class: CostClass::L, deterministic: true  },
    LevelDescriptor { id: "ast.items",          spec_number: 8,  cost_class: CostClass::L, deterministic: true  },
    LevelDescriptor { id: "ast.expanded",       spec_number: 9,  cost_class: CostClass::B, deterministic: false },
    LevelDescriptor { id: "ast.resolved",       spec_number: 10, cost_class: CostClass::B, deterministic: true  },
    // §2.3 Compiler-internal IRs
    LevelDescriptor { id: "hir",                spec_number: 11, cost_class: CostClass::B, deterministic: true  },
    LevelDescriptor { id: "thir",               spec_number: 12, cost_class: CostClass::B, deterministic: true  },
    LevelDescriptor { id: "mir.preopt",         spec_number: 13, cost_class: CostClass::B, deterministic: true  },
    LevelDescriptor { id: "mir.opt",            spec_number: 14, cost_class: CostClass::B, deterministic: true  },
    LevelDescriptor { id: "borrowck",           spec_number: 15, cost_class: CostClass::B, deterministic: true  },
    // §2.4 Type-system derived views
    LevelDescriptor { id: "type.usage",         spec_number: 16, cost_class: CostClass::L, deterministic: true  },
    LevelDescriptor { id: "type.trait_impl",    spec_number: 17, cost_class: CostClass::B, deterministic: true  },
    LevelDescriptor { id: "type.mono",          spec_number: 18, cost_class: CostClass::B, deterministic: true  },
    LevelDescriptor { id: "type.coherence",     spec_number: 19, cost_class: CostClass::B, deterministic: true  },
    // §2.5 Codegen
    LevelDescriptor { id: "llvm.preopt",        spec_number: 20, cost_class: CostClass::C, deterministic: true  },
    LevelDescriptor { id: "llvm.opt",           spec_number: 21, cost_class: CostClass::C, deterministic: true  },
    LevelDescriptor { id: "asm",                spec_number: 22, cost_class: CostClass::C, deterministic: true  },
    LevelDescriptor { id: "obj",                spec_number: 23, cost_class: CostClass::C, deterministic: true  },
    LevelDescriptor { id: "bin",                spec_number: 24, cost_class: CostClass::C, deterministic: true  },
    // §2.6 Binary-artifact
    LevelDescriptor { id: "sym.table",          spec_number: 25, cost_class: CostClass::C, deterministic: true  },
    LevelDescriptor { id: "sym.demangled",      spec_number: 26, cost_class: CostClass::L, deterministic: true  },
    LevelDescriptor { id: "sym.sections",       spec_number: 27, cost_class: CostClass::C, deterministic: true  },
    LevelDescriptor { id: "sym.relocs",         spec_number: 28, cost_class: CostClass::C, deterministic: true  },
    LevelDescriptor { id: "dwarf",              spec_number: 29, cost_class: CostClass::C, deterministic: true  },
    LevelDescriptor { id: "link.map",           spec_number: 30, cost_class: CostClass::C, deterministic: true  },
    // §2.7 Graph-derived
    LevelDescriptor { id: "graph.mod",          spec_number: 31, cost_class: CostClass::L, deterministic: true  },
    LevelDescriptor { id: "graph.call_static",  spec_number: 32, cost_class: CostClass::L, deterministic: true  },
    LevelDescriptor { id: "graph.call_mono",    spec_number: 33, cost_class: CostClass::L, deterministic: true  },
    LevelDescriptor { id: "graph.cfg",          spec_number: 34, cost_class: CostClass::L, deterministic: true  },
    LevelDescriptor { id: "graph.dataflow",     spec_number: 35, cost_class: CostClass::L, deterministic: true  },
    LevelDescriptor { id: "graph.type_usage",   spec_number: 36, cost_class: CostClass::L, deterministic: true  },
    LevelDescriptor { id: "graph.sccs",         spec_number: 37, cost_class: CostClass::L, deterministic: true  },
    LevelDescriptor { id: "graph.communities",  spec_number: 38, cost_class: CostClass::L, deterministic: true  },
    LevelDescriptor { id: "graph.power_law",    spec_number: 39, cost_class: CostClass::L, deterministic: true  },
    LevelDescriptor { id: "graph.self_sim",     spec_number: 40, cost_class: CostClass::L, deterministic: true  },
    // §2.8 Project-level metadata
    LevelDescriptor { id: "cargo.workspace",    spec_number: 41, cost_class: CostClass::F, deterministic: true  },
    LevelDescriptor { id: "cargo.lock",         spec_number: 42, cost_class: CostClass::F, deterministic: true  },
    LevelDescriptor { id: "cargo.cfg",          spec_number: 43, cost_class: CostClass::F, deterministic: true  },
    LevelDescriptor { id: "cargo.toolchain",    spec_number: 44, cost_class: CostClass::F, deterministic: true  },
    // §2.9 External-tool derived
    LevelDescriptor { id: "clippy",             spec_number: 45, cost_class: CostClass::B, deterministic: true  },
    LevelDescriptor { id: "rustdoc",            spec_number: 46, cost_class: CostClass::B, deterministic: true  },
    LevelDescriptor { id: "rustfmt",            spec_number: 47, cost_class: CostClass::F, deterministic: true  },
    LevelDescriptor { id: "tests",              spec_number: 48, cost_class: CostClass::B, deterministic: true  },
    LevelDescriptor { id: "doctests",           spec_number: 49, cost_class: CostClass::B, deterministic: true  },
];

/// Look up the descriptor for a level id, if it is in the required-levels
/// table.
pub fn descriptor_for(id: &str) -> Option<&'static LevelDescriptor> {
    REQUIRED_LEVELS.iter().find(|d| d.id == id)
}

/// Per-run configuration. Mirrors the CLI's user-controllable flags.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub enable_rustc_internal: bool,
    pub toolchain_override: Option<String>,
    /// Opt-in for spec §2.5 codegen levels (20–24). These drive `cargo
    /// rustc` and parse LLVM IR / assembly / object output, so they're
    /// the first stage that pays a real build cost. Off by default; the
    /// five codegen levels skip cleanly with a documented reason when
    /// not enabled.
    pub enable_codegen: bool,
}

/// What each [`Level::extract`] call receives.
#[derive(Debug, Clone)]
pub struct Context {
    pub project_root: PathBuf,
    pub config: Config,
}

/// What a successful [`Level::extract`] produces.
///
/// `vector` is a list of `(dotted_key, value)` pairs the level wishes to
/// add to the flat namespace. `entities` is per-entity records grouped by
/// `<level>.<entity_kind>` (e.g. `"ast.func"`), per spec §6.
#[derive(Debug, Default, Clone)]
pub struct LevelOutput {
    pub vector: Vec<(String, f64)>,
    pub entities: BTreeMap<String, Vec<EntityRecord>>,
}

/// Non-fatal level outcomes. `Skipped` means "prerequisite missing,
/// consumer should treat keys as absent"; `Failed` means "prerequisite
/// present but extraction blew up". Both produce a `reason` in the §7
/// report and do not abort the run.
#[derive(Debug, thiserror::Error)]
pub enum LevelError {
    #[error("{0}")]
    Skipped(String),
    #[error("{0}")]
    Failed(String),
}

/// One representation level. Spec §12.
pub trait Level: Send + Sync {
    fn id(&self) -> &'static str;
    fn cost_class(&self) -> CostClass;
    fn deterministic(&self) -> bool;
    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError>;
}

/// Fatal errors that abort the entire run before producing an envelope
/// (spec §7: "exits non-zero only if the orchestrator itself cannot
/// proceed").
#[derive(Debug, thiserror::Error)]
pub enum OrchestrateError {
    #[error("project root {path:?} does not exist")]
    ProjectRootMissing { path: PathBuf },
    #[error("project root {path:?} is not a directory (pass the project's root directory, not a file)")]
    ProjectRootNotADirectory { path: PathBuf },
    #[error("project root {path:?} is not a Cargo project (no Cargo.toml found)")]
    NotACargoProject { path: PathBuf },
    #[error("could not probe rustc toolchain: {0}")]
    RustcProbeFailed(String),
    #[error("vector key {key:?} claimed by both level {first:?} and level {second:?}")]
    NamespaceCollision { key: String, first: String, second: String },
    #[error("level id {0:?} is not in the required-levels table (typo or unmapped level?)")]
    UnknownLevel(String),
    #[error("level id {0:?} was registered more than once")]
    DuplicateLevel(String),
}

/// Owns the registered levels and produces an [`OracleOutput`] when run.
///
/// `Runner` is intentionally dumb: register levels, call `run`. Higher-level
/// orchestration (caching, parallelism) is out of scope for v1 — the rustc
/// subprocess and codegen dominate wall-clock and aren't safely parallelisable
/// within a single project.
pub struct Runner {
    levels: Vec<Box<dyn Level>>,
    cstat_version: String,
}

impl Runner {
    pub fn new() -> Self {
        Self {
            levels: Vec::new(),
            cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    /// Override the `cstat_version` stamped into the envelope. The CLI sets
    /// this to its own package version so the user-visible tool's version
    /// is what shows up in the output.
    pub fn with_cstat_version(mut self, version: impl Into<String>) -> Self {
        self.cstat_version = version.into();
        self
    }

    pub fn register<L: Level + 'static>(&mut self, level: L) -> &mut Self {
        self.levels.push(Box::new(level));
        self
    }

    /// Execute the run. See [`OrchestrateError`] for the conditions that
    /// produce a fatal error (per-level failures are observable in the
    /// envelope, not fatal).
    pub fn run(&self, ctx: &Context) -> Result<OracleOutput, OrchestrateError> {
        if !ctx.project_root.exists() {
            return Err(OrchestrateError::ProjectRootMissing {
                path: ctx.project_root.clone(),
            });
        }
        if !ctx.project_root.is_dir() {
            return Err(OrchestrateError::ProjectRootNotADirectory {
                path: ctx.project_root.clone(),
            });
        }
        if !ctx.project_root.join("Cargo.toml").is_file() {
            return Err(OrchestrateError::NotACargoProject {
                path: ctx.project_root.clone(),
            });
        }

        self.validate_registrations()?;

        let toolchain = probe_toolchain(&ctx.project_root, &ctx.config)
            .map_err(OrchestrateError::RustcProbeFailed)?;

        let mut envelope = OracleOutput::empty(self.cstat_version.clone(), toolchain);
        envelope.config_digest = compute_config_digest(&ctx.config);

        let wall_start = Instant::now();

        // Tracks which level owns each emitted vector key, for the
        // namespace cross-check (anchor test 5).
        let mut owner_of: BTreeMap<String, String> = BTreeMap::new();
        let registered: HashSet<&str> = self.levels.iter().map(|l| l.id()).collect();

        for level in &self.levels {
            let descriptor = *descriptor_for(level.id())
                .expect("validate_registrations ensures every registered id is known");
            let level_start = Instant::now();
            let result = level.extract(ctx);
            let elapsed = level_start.elapsed().as_secs_f64();

            let report = match result {
                Ok(output) => {
                    for (key, value) in output.vector {
                        if let Some(prior) = owner_of.get(&key) {
                            return Err(OrchestrateError::NamespaceCollision {
                                key,
                                first: prior.clone(),
                                second: level.id().to_string(),
                            });
                        }
                        owner_of.insert(key.clone(), level.id().to_string());
                        envelope.vector.insert(key.clone(), value);
                        envelope.provenance.record(key, level.id());
                    }
                    for (kind, mut records) in output.entities {
                        envelope
                            .entities
                            .entry(kind)
                            .or_default()
                            .append(&mut records);
                    }
                    LevelReport {
                        status: LevelStatus::Ok,
                        reason: None,
                        recompute_cost_class: descriptor.cost_class,
                        deterministic: descriptor.deterministic,
                        extraction_seconds: elapsed,
                    }
                }
                Err(LevelError::Skipped(reason)) => LevelReport {
                    status: LevelStatus::Skipped,
                    reason: Some(reason),
                    recompute_cost_class: descriptor.cost_class,
                    deterministic: descriptor.deterministic,
                    extraction_seconds: elapsed,
                },
                Err(LevelError::Failed(reason)) => LevelReport {
                    status: LevelStatus::Failed,
                    reason: Some(reason),
                    recompute_cost_class: descriptor.cost_class,
                    deterministic: descriptor.deterministic,
                    extraction_seconds: elapsed,
                },
            };
            envelope.levels.insert(level.id().to_string(), report);
        }

        // Auto-fill: every required level not covered by a registered
        // Level appears as skipped/"not implemented". The contract from
        // spec §4 and anchor test 2: no required level is silently absent.
        for descriptor in REQUIRED_LEVELS {
            if !registered.contains(descriptor.id) {
                envelope.levels.insert(
                    descriptor.id.to_string(),
                    LevelReport {
                        status: LevelStatus::Skipped,
                        reason: Some("not implemented".to_string()),
                        recompute_cost_class: descriptor.cost_class,
                        deterministic: descriptor.deterministic,
                        extraction_seconds: 0.0,
                    },
                );
            }
        }

        // meta.* scalars (spec §8). Always present so consumers can detect
        // degraded runs without scanning every key. `files_unparseable` is
        // derived from `entities.ast.file_errors` so AST levels never have
        // to touch the meta namespace directly.
        let levels_missing = envelope
            .levels
            .values()
            .filter(|r| !matches!(r.status, LevelStatus::Ok))
            .count() as f64;
        emit_meta_scalar(&mut envelope, &mut owner_of, "meta.levels_missing.count", levels_missing)?;
        let files_unparseable = envelope
            .entities
            .get("ast.file_errors")
            .map(|v| v.len())
            .unwrap_or(0) as f64;
        emit_meta_scalar(
            &mut envelope,
            &mut owner_of,
            "meta.files_unparseable.count",
            files_unparseable,
        )?;

        envelope.wall_clock_seconds = wall_start.elapsed().as_secs_f64();
        Ok(envelope)
    }

    fn validate_registrations(&self) -> Result<(), OrchestrateError> {
        let known: HashSet<&'static str> = REQUIRED_LEVELS.iter().map(|d| d.id).collect();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for level in &self.levels {
            let id = level.id();
            if !known.contains(id) {
                return Err(OrchestrateError::UnknownLevel(id.to_string()));
            }
            if !seen.insert(id) {
                return Err(OrchestrateError::DuplicateLevel(id.to_string()));
            }
        }
        Ok(())
    }
}

impl Default for Runner {
    fn default() -> Self {
        Self::new()
    }
}

fn emit_meta_scalar(
    envelope: &mut OracleOutput,
    owner_of: &mut BTreeMap<String, String>,
    key: &str,
    value: f64,
) -> Result<(), OrchestrateError> {
    if let Some(prior) = owner_of.get(key) {
        return Err(OrchestrateError::NamespaceCollision {
            key: key.to_string(),
            first: prior.clone(),
            second: "meta".to_string(),
        });
    }
    owner_of.insert(key.to_string(), "meta".to_string());
    envelope.vector.insert(key.to_string(), value);
    envelope.provenance.record(key, "meta");
    Ok(())
}

fn probe_toolchain(project_root: &Path, config: &Config) -> Result<Toolchain, String> {
    // Run rustc from inside the project so that rustup honours any
    // `rust-toolchain.toml` pin in the user's tree — that's the contract
    // spec §9 names. With no pin in sight this falls back to the default
    // toolchain on PATH, which is what we want.
    let output = Command::new("rustc")
        .arg("--version")
        .arg("--verbose")
        .current_dir(project_root)
        .output()
        .map_err(|e| format!("could not invoke rustc: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "rustc exited with {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut summary_version = String::new();
    let mut release = String::new();
    let mut host = String::new();
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("rustc ") {
            if summary_version.is_empty() {
                summary_version = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_string();
            }
        } else if let Some(rest) = line.strip_prefix("release: ") {
            release = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("host: ") {
            host = rest.trim().to_string();
        }
    }

    let version = if !release.is_empty() { release } else { summary_version.clone() };
    let channel = config
        .toolchain_override
        .clone()
        .unwrap_or_else(|| classify_channel(&summary_version));

    Ok(Toolchain { channel, version, host })
}

fn classify_channel(version: &str) -> String {
    if version.contains("-nightly") {
        "nightly".to_string()
    } else if version.contains("-beta") {
        "beta".to_string()
    } else if version.contains("-dev") {
        "dev".to_string()
    } else {
        "stable".to_string()
    }
}

/// FNV-1a 64 over a canonical text rendering of the config. Deterministic
/// across runs and hosts; no external crate dependency for stage 2.
fn compute_config_digest(config: &Config) -> String {
    let canonical = format!(
        "enable_rustc_internal={};enable_codegen={};toolchain={}",
        config.enable_rustc_internal,
        config.enable_codegen,
        config.toolchain_override.as_deref().unwrap_or("")
    );
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in canonical.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{:016x}", hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/single-file-lib")
    }

    fn ctx_for(path: PathBuf) -> Context {
        Context {
            project_root: path,
            config: Config::default(),
        }
    }

    struct FakeLevel {
        id: &'static str,
        output: LevelOutput,
    }
    impl Level for FakeLevel {
        fn id(&self) -> &'static str { self.id }
        fn cost_class(&self) -> CostClass { CostClass::F }
        fn deterministic(&self) -> bool { true }
        fn extract(&self, _: &Context) -> Result<LevelOutput, LevelError> {
            Ok(self.output.clone())
        }
    }

    #[test]
    fn empty_runner_marks_every_required_level_as_skipped_not_implemented() {
        let envelope = Runner::new().run(&ctx_for(fixture_path())).unwrap();

        // Spec §4: every required level appears.
        assert_eq!(envelope.levels.len(), REQUIRED_LEVELS.len());
        for descriptor in REQUIRED_LEVELS {
            let report = envelope
                .levels
                .get(descriptor.id)
                .unwrap_or_else(|| panic!("missing level {}", descriptor.id));
            assert_eq!(report.status, LevelStatus::Skipped, "{}", descriptor.id);
            assert_eq!(
                report.reason.as_deref(),
                Some("not implemented"),
                "{}",
                descriptor.id
            );
            // Descriptor metadata propagates onto the report.
            assert_eq!(report.recompute_cost_class, descriptor.cost_class);
            assert_eq!(report.deterministic, descriptor.deterministic);
        }

        // meta.* scalar is present and consistent with the level-status count.
        let missing = envelope
            .vector
            .get("meta.levels_missing.count")
            .expect("meta.levels_missing.count missing");
        assert_eq!(missing as usize, REQUIRED_LEVELS.len());
    }

    #[test]
    fn cross_check_rejects_two_levels_claiming_the_same_vector_key() {
        let mut runner = Runner::new();
        runner
            .register(FakeLevel {
                id: "fs.layout",
                output: LevelOutput {
                    vector: vec![("shared.key".into(), 1.0)],
                    entities: BTreeMap::new(),
                },
            })
            .register(FakeLevel {
                id: "fs.bytes",
                output: LevelOutput {
                    vector: vec![("shared.key".into(), 2.0)],
                    entities: BTreeMap::new(),
                },
            });
        let err = runner.run(&ctx_for(fixture_path())).unwrap_err();
        match err {
            OrchestrateError::NamespaceCollision { key, first, second } => {
                assert_eq!(key, "shared.key");
                assert_eq!(first, "fs.layout");
                assert_eq!(second, "fs.bytes");
            }
            other => panic!("expected NamespaceCollision, got {other:?}"),
        }
    }

    #[test]
    fn unknown_level_id_is_rejected_before_run_begins() {
        let mut runner = Runner::new();
        runner.register(FakeLevel {
            id: "definitely-not-a-real-level",
            output: LevelOutput::default(),
        });
        let err = runner.run(&ctx_for(fixture_path())).unwrap_err();
        assert!(matches!(err, OrchestrateError::UnknownLevel(_)));
    }

    #[test]
    fn duplicate_registration_is_rejected() {
        let mut runner = Runner::new();
        runner
            .register(FakeLevel {
                id: "fs.layout",
                output: LevelOutput::default(),
            })
            .register(FakeLevel {
                id: "fs.layout",
                output: LevelOutput::default(),
            });
        let err = runner.run(&ctx_for(fixture_path())).unwrap_err();
        assert!(matches!(err, OrchestrateError::DuplicateLevel(_)));
    }

    #[test]
    fn registered_ok_level_does_not_inflate_levels_missing_count() {
        let mut runner = Runner::new();
        runner.register(FakeLevel {
            id: "fs.layout",
            output: LevelOutput {
                vector: vec![("fs.global.file_count".into(), 1.0)],
                entities: BTreeMap::new(),
            },
        });
        let envelope = runner.run(&ctx_for(fixture_path())).unwrap();
        let missing = envelope
            .vector
            .get("meta.levels_missing.count")
            .unwrap();
        // 49 required levels minus the one registered OK level.
        assert_eq!(missing as usize, REQUIRED_LEVELS.len() - 1);
        // The OK level's report reflects status=ok.
        assert_eq!(
            envelope.levels.get("fs.layout").unwrap().status,
            LevelStatus::Ok
        );
        // Provenance attributes the key to the level that emitted it.
        assert_eq!(
            envelope
                .provenance
                .0
                .get("fs.global.file_count")
                .map(|v| v.as_slice()),
            Some(["fs.layout".to_string()].as_slice())
        );
    }

    #[test]
    fn missing_project_root_is_a_fatal_error() {
        let runner = Runner::new();
        let ctx = ctx_for(PathBuf::from("/nonexistent/path/for/cstat/test"));
        let err = runner.run(&ctx).unwrap_err();
        assert!(matches!(err, OrchestrateError::ProjectRootMissing { .. }));
    }

    #[test]
    fn directory_without_cargo_toml_is_a_fatal_error() {
        let tmp = std::env::temp_dir().join(format!(
            "cstat-orchestrate-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let runner = Runner::new();
        let err = runner.run(&ctx_for(tmp.clone())).unwrap_err();
        fs::remove_dir_all(&tmp).ok();
        assert!(matches!(err, OrchestrateError::NotACargoProject { .. }));
    }

    #[test]
    fn pointing_project_root_at_a_file_is_a_fatal_error() {
        let tmp = std::env::temp_dir().join(format!(
            "cstat-orchestrate-test-file-{}",
            std::process::id()
        ));
        fs::write(&tmp, b"not a directory").unwrap();
        let runner = Runner::new();
        let err = runner.run(&ctx_for(tmp.clone())).unwrap_err();
        fs::remove_file(&tmp).ok();
        assert!(matches!(
            err,
            OrchestrateError::ProjectRootNotADirectory { .. }
        ));
    }
}
