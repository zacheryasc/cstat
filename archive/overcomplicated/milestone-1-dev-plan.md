# Milestone 1 Development Plan — Representation Extraction

This document is the implementation plan for Milestone 1 of the roadmap.
The roadmap names it "Define the Representation Stack"; in practice the
work is **build a tool that observes every representation level the spec
names, for any Rust project, and emits the result as one JSON artifact.**
That artifact is the coordinate system every later milestone is built on.

The spec is `milestone-1-representation-stack.md`. This plan does not
restate it; it explains how to deliver it.

## What this milestone produces

A single CLI:

```
cstat extract --path <rust-project> --output state.json
```

`state.json` matches §7 of the spec exactly: a `vector` of dotted-key
scalars, per-entity records under `entities`, level-by-level status,
provenance, schema version, toolchain stamp. Every representation level
in §2.1–§2.9 of the spec (excluding §2.10) has working extraction code
backing it.

Out of scope (be explicit so we don't drift):

- No moves, no optimizer, no policy decisions about what to change.
- No human-facing summary, no scoring, no "is this codebase healthy"
  judgment. JSON is the output.
- No behavioral testing — that's Milestone 4.
- Rust only. Host triple only. Debug build only (release is a flag we
  surface but don't tune for).
- No caching layer beyond what cargo and rustc do natively. If
  re-extraction is slow we'll address it after we see the numbers.

## Architectural commitments

These are the load-bearing decisions. Everything downstream assumes them.

**Greenfield code.** Do not try to preserve the existing cstat
subcommands (`dump`, `complexity`, `deps`, `flow`, etc.). They were
useful as a probe; they are not the design target. Write what the spec
calls for and let the old surface fall away.

**rustc isolation boundary.** rustc-internal IRs (HIR, THIR, MIR,
borrowck, mono-items) are accessed via `rustc_driver` running in a
separate nightly sub-binary. That binary translates rustc's internal
data structures into our own plain serde-friendly mirror types and
writes JSON. The rest of the workspace never sees a `TyCtxt`, never
sees a `'tcx` lifetime, and stays on stable Rust. When rustc churns,
exactly one file changes — the translator — and the rest of the system
is unaffected. This is the whole point of the boundary.

**One workspace, multiple crates** — separation by stability surface,
not by feature:

```
crates/
  cstat-schema/         stable. §5/§6/§7 types, JSON serde.
  cstat-ir/             stable. Mirror types for HIR/THIR/MIR/borrowck.
  cstat-orchestrate/    stable. Level trait, runner, output assembly.
  cstat-extract-stable/ stable. Every level not needing rustc internals.
  cstat-extract-rustc/  NIGHTLY. rustc_driver-driven extractor binary.
  cstat-cli/            stable. CLI entry point.
```

A `rust-toolchain.toml` inside `cstat-extract-rustc/` pins it to nightly;
the workspace root stays on stable. Cargo respects per-directory toolchains.

**Output schema is the contract.** The JSON shape in §7 of the spec is
the API. Every later milestone reads it. Schema-version per §10. Don't
"clean it up" mid-implementation; if it needs changing, bump the version
and update the spec.

---

## Phase 1 — Scaffolding

Goal: workspace exists, schema types compile, CLI runs end-to-end with
zero levels registered, and the resulting JSON validates against §7.
Nothing measures anything yet — but the envelope is real.

### `cstat-schema`

The serializable surface. Pure data types, serde derive throughout.

- `Vector` — `BTreeMap<String, f64>` newtype. BTreeMap so iteration is
  deterministic for snapshot tests.
- `EntityRecord { ids: IdSet, metrics: BTreeMap<String, f64> }`.
- `IdSet { file: Option<FileId>, module: Option<ModuleId>,
  ast_func: Option<AstFuncId>, symbol: Option<SymbolId>,
  mono: Option<MonoId> }`. Per §6, every entity carries whichever IDs
  it's known by.
- IDs are plain structs; `AstFuncId` includes a structural hash of the
  AST item so it survives reformatting but not real edits.
- `OracleOutput` matching §7 field-for-field: `schema_version`,
  `cstat_version`, `toolchain`, `config_digest`, `wall_clock_seconds`,
  `levels` (status per level), `vector`, `entities`, `provenance`.
- Schema version starts at `0.1.0`. Bump rules per §10.

### `cstat-orchestrate`

The runner. Knows nothing about specific levels — only the trait.

```rust
trait Level: Send + Sync {
    fn id(&self) -> &'static str;
    fn deterministic(&self) -> bool;
    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError>;
}
```

`Context` carries the project root, the user's config, and a
`SharedArtifacts` cache. `SharedArtifacts` is a lazy memoizer for
expensive intermediate products that multiple levels read — `cargo
metadata` output, the LLVM IR text, the parsed object file, the
`cstat-ir` dump from the nightly extractor. First level to ask pays
the cost; the rest hit a cached reference. Keep this dumb: a few
`OnceCell`s in a struct is enough.

`LevelOutput` carries the level's contribution to the vector
(`Vec<(String, f64)>`), per-entity records, and optionally the set of
key prefixes it claims to own. The runner cross-checks claims against
emissions and fails loudly on collision — this is how we keep the
namespace honest without a central registry.

Runner runs levels sequentially in v1. Don't parallelize until we
measure that we need it; the rustc subprocess and codegen dominate
wall-clock time and they're not parallelizable inside one project
anyway.

### `cstat-cli`

One subcommand: `extract --path <PATH> [--output <FILE>]
[--enable-rustc-internal] [--toolchain <CHANNEL>]`. Writes JSON to file
or stdout. No pretty-printer in v1 — debugging is `jq`.

### Completion gate

A toy Rust project (check in `examples/single-file-lib/` from §13) runs
through `cstat extract` and produces a JSON file. All levels report
`status: "skipped"` with reason `"not implemented"`, but the envelope
parses against `OracleOutput`. `cargo test` in the workspace passes.

---

## Phase 2 — Stable-Rust extractors

Goal: every level reachable without rustc internals has a working
`Level` impl in `cstat-extract-stable`. This is the bulk of the work
by volume but most of it is mechanical — pick the right crate, walk
the right tree, emit the right keys.

Group the work by the file or artifact each level reads from. Levels
in the same group share intermediate state via `SharedArtifacts`.

**Group A — Filesystem and source text** (levels 1–6, plus 47).
One pass walks the project tree once, holds each file's bytes briefly,
forks downstream into: filesystem stats, raw-byte stats, line-level
stats, tokens (via `proc_macro2`), comments (token filter), whitespace
(rustfmt diff via `cargo fmt -- --check`). Don't re-read files.

**Group B — Syntactic AST** (levels 7, 8, 9).
Level 7: `syn::parse_file` per `.rs` file; parse errors recorded under
`entities.ast.file_errors` and the file excluded from AST metrics per
§11. Level 8: walk the parsed CSTs to assemble the module skeleton and
visibility distribution. Level 9: shell out to `cargo expand` (preferred
because it's stable) for macro-expanded source, then re-run level 7's
pass over the expanded text. The level-9 metrics are the same as level
7's; what's interesting is the delta.

**Group C — Cargo and project metadata** (levels 41–44).
Direct calls to `cargo metadata --format-version 1`, `rustc --print
cfg`, and direct file reads of `Cargo.lock` and `rust-toolchain.toml`.
This group is essentially free; do it early so the toolchain stamp is
available for the output envelope.

**Group D — Codegen** (levels 20–24).
Invoke `cargo rustc -- --emit=llvm-ir -C no-prepopulate-passes` for
the pre-opt IR (level 20), `--emit=llvm-ir` for the optimized form
(level 21), `--emit=asm` for assembly (level 22). The build also
produces object files (level 23) and a linked binary (level 24) which
the orchestrator locates via `cargo build --message-format=json`. LLVM
IR is parsed with the `llvm-ir` crate (text-format reader). Per-function
metrics: instruction count, basic-block count, instruction-kind
histogram, call-site count.

**Group E — Binary artifacts** (levels 25–30).
The `object` crate handles 25 (symbol table), 27 (sections), 28
(relocations). `rustc-demangle` runs over level 25 to produce 26 (the
demangled view, from which source-to-symbol ratio falls out). Level 29
uses `gimli` over the debug build's DWARF — this is where the
AST↔Symbol identity join from §6 becomes possible, so the join logic
lives here. Level 30 parses the linker map file (`-Wl,-Map=...`).

**Group F — Type-system views** (levels 16, 17, 19).
Level 16 is the AST-approximation in v1: walk syn types per function,
emit per-function distinct-type count. Levels 17 and 19 come from
rustdoc JSON (`cargo +nightly rustdoc -- --output-format json`), parsed
with the `rustdoc-types` crate. Yes, rustdoc JSON is nightly-only — but
it's a separate process, not in-process rustc, so it doesn't compromise
the isolation boundary. Treat it like any other tool invocation.

**Group G — Graph derivations** (levels 31, 32, 34, 36–40).
Build module dependency graph (31) and intra-crate call graph (32)
from Group B's name-unresolved AST. CFG-per-function (34) walks each
function body's syn tree into a basic-block graph (pre-MIR; MIR's CFG
arrives in Phase 3 as a separate level). Type-usage graph (36) lifts
Group F's level-16 output to a graph. SCCs (37) via Tarjan. Communities
(38) via Louvain or label-propagation, with modularity Q emitted. Power-
law fit (39) is a utility: takes any graph, fits a degree-distribution
exponent via MLE plus a KS goodness-of-fit. Self-similarity (40) takes
any per-entity scalar and reports the distribution distance (KS) across
aggregation scales (function → file → module).

**Group H — External tools** (levels 45, 46, 48, 49).
`cargo clippy --message-format=json` for 45. Group F already does 46.
`cargo test -- --list --format json -Z unstable-options` for 48; the
doc-test variant for 49.

### Completion gate

Run `cstat extract` on the §13 worked example. Every level in this
phase reports `status: "ok"`. The vector contains keys for each of
those levels. Approximate value match against §13 is fine — the spec
example was illustrative, and we can correct §13's numbers once we
have ground truth. What must hold: shape matches, IDs join where §13
shows them joining (the AST↔Symbol example), provenance is populated.

---

## Phase 3 — rustc_driver extractor (the isolated nightly binary)

Goal: levels 10–15 and the exact form of 18, accessed in-process via
rustc, exposed to the rest of the system as plain JSON mirror types.

### The boundary

`cstat-ir` defines the mirror types. One module per IR layer:
`cstat_ir::hir`, `cstat_ir::thir`, `cstat_ir::mir`, `cstat_ir::borrowck`,
`cstat_ir::monoitems`. Each module has plain serde-derive structs
capturing the fields we measure — *not* a full mirror of rustc's
internals. Enums use string discriminants for forward-compat with rustc
shape changes. No lifetimes, no rustc deps, no internal types leak
through.

A minimal example shape (final form decided during Phase 3):

```rust
// cstat_ir::mir
pub struct Body {
    pub def_path: String,
    pub arg_count: u32,
    pub local_count: u32,
    pub basic_blocks: Vec<BasicBlock>,
}

pub struct BasicBlock {
    pub id: u32,
    pub statement_kinds: Vec<String>, // string discriminants
    pub terminator: String,
    pub predecessors: Vec<u32>,
    pub successors: Vec<u32>,
}
```

The orchestrator and `cstat-extract-stable` may depend on `cstat-ir`.
Neither depends on rustc.

### The nightly binary

`cstat-extract-rustc` is its own bin crate, pinned to nightly via
`rust-toolchain.toml` in that crate's directory.

Architecture:
- Parse args: project path, output path, which IR levels to emit.
- Construct an `rustc_driver::RunCompiler` with our `Callbacks` impl.
- The `after_analysis` callback has `TyCtxt`. Walk the bodies we need,
  call the per-level translator, accumulate `cstat-ir` values.
- Write JSON to the output path. Exit.

The orchestrator's Level impl for rustc-derived levels invokes this
binary as a subprocess. Subprocess failure (no nightly installed, rustc
panic, project doesn't build) → each affected level reports `status:
"failed"` with reason. The rest of the run continues.

### Translators

One per IR layer, all living in `cstat-extract-rustc`:

- `translate_hir(tcx)` — produces `cstat_ir::hir::Item` records.
- `translate_thir(tcx, body)` — typed HIR per body; we emit
  per-expression type info aggregated into per-function metrics
  (polymorphism density, generic-arg counts, trait-object density).
- `translate_mir(body)` — `cstat_ir::mir::Body`. Run twice: once on
  pre-opt MIR (level 13), once on optimized MIR (level 14). The delta
  is its own observable.
- `translate_borrowck(tcx, def_id)` — region/lifetime constraint
  count, NLL conflict density.
- `translate_monoitems(tcx)` — `tcx.collect_and_partition_mono_items`
  rolled up to per-generic-item instantiation count.

### Metric extraction

Lives back in `cstat-extract-stable` (or a sibling stable crate; pick
during implementation). Takes `cstat-ir` values and emits
`(vector_key, f64)` pairs. Pure functions, unit-testable against
hand-built mirror values — this is the natural place for property tests
(see Testing).

### Levels covered

- 10 (name-resolved AST): import graph, shadowing structure from HIR.
- 11 (HIR): branch counts, expression-kind histogram, sugar density.
- 12 (THIR): type information rolled into metrics.
- 13 (MIR pre-opt): per-function basic-block count, statement count,
  terminator-kind histogram.
- 14 (MIR opt): the same, after MIR optimization passes.
- 15 (borrowck): lifetime constraint count, region count, NLL density.
- 18 (generic instantiation, exact): from mono-items.

### Completion gate

Subprocess runs on the §13 example, emits a `cstat-ir` JSON, the
orchestrator distributes it into the vector. All nightly levels show
`status: "ok"` when nightly is installed and `"skipped"` with reason
`"nightly toolchain unavailable"` otherwise.

---

## Phase 4 — Derived levels + validation

Goal: the levels that compose other levels' outputs, plus the test
harness that proves the whole thing reproduces.

### Derived levels (33, 35)

- Level 33 (inter-crate / monomorph call graph): joins level 26
  (demangled symbols) with level 20 (LLVM IR `call` instructions
  resolved to callee symbols). Output: a graph keyed by `MonoId`.
- Level 35 (data-flow per function): from level 20's SSA form,
  build def-use chains; emit per-function dataflow density.

These are pure compositions over previously extracted artifacts; they
don't invoke any new tooling.

### Validation harness

Lives in `cstat-orchestrate/tests/` and `cstat-cli/tests/`. Per
preferences: scenario tests, property tests, contract tests — no
white-box "insert-then-lookup" style.

- **Worked-example snapshot.** The §13 toy crate is checked in under
  `examples/single-file-lib/`. A scenario test runs `cstat extract`
  on it and snapshots the resulting JSON. The snapshot is reviewed
  on diff. Numeric values in §13 may need correction the first time
  this runs; that's a spec edit, not a code bug.
- **Determinism property test.** For any project in the examples
  directory, two runs in a row produce byte-identical JSON for every
  key whose owning level declares `deterministic: true`. Non-
  deterministic keys are inspected separately.
- **Failure-mode contract tests.** One small fixture project per row
  of §11: an unparseable `.rs` file, a missing `Cargo.toml`, a panicky
  proc macro (if one can be constructed), a missing nightly toolchain
  (simulated by an env var the runner respects). Each fixture asserts
  the expected `status` and `reason` shape — not the internal control
  flow.
- **Schema roundtrip property test.** `OracleOutput → JSON → Value →
  OracleOutput` is lossless for any output the runner can produce.
  Catches accidental schema drift early.

### Real-target smoke

Run `cstat extract` against itself (this workspace) and at least one
external Rust crate (a small one — `serde_json`, `ripgrep`, or
similar). The vector should populate for every required level with
no `failed` statuses. This is exploratory, not a pass/fail gate, but
the first time we see the numbers will shape what we want to add or
adjust in §5's aggregate set.

### Completion gate

All four test categories pass. `cstat extract` on at least two real
crates produces a complete JSON with no `failed` statuses (only
`skipped` where genuinely unavailable, e.g., missing optional tooling
on the dev machine).

---

## Testing strategy in one paragraph

No white-box tests. No "construct an AST, call the metric, assert the
metric returns what the AST says." Tests fall into three buckets:
**scenario** (run the tool on a real or fixture project, assert
properties of the output), **property** (determinism, schema
roundtrip, key-namespace non-collision), **contract** (failure-mode
behavior matches §11's catalog). Tests should survive a refactor of
any individual level's internals — they observe the JSON, not the
code. The §13 worked example doubles as our primary scenario fixture
and as the spec's anchor; if the spec's numbers change, the snapshot
changes with them.

## Environment prerequisites

- Stable Rust (workspace default).
- Nightly Rust installed alongside (for `cstat-extract-rustc`,
  `cargo expand`, rustdoc JSON, `cargo test --list -Z`). Pinned via
  `rust-toolchain.toml` inside the crate that needs it.
- `cargo` on PATH.
- Standard system tools: `cargo`, `rustc`, `rustup`, and a linker.
  Object/DWARF/symbol parsing is all in-process via crates (`object`,
  `gimli`, `rustc-demangle`); no `nm`/`objdump` shell-outs.
- Optional: `cargo-expand` (a cargo subcommand); the orchestrator
  detects absence and falls back / skips.

## Sequencing and parallelism

Phases are listed in dependency order. Phase 0 must land before any
code so the spec is stable. Phase 1 must land before Phases 2–4 (they
need the scaffold). Phases 2 and 3 are mostly independent — different
crates, different toolchains, different problems — and can be worked
in parallel by different agents if convenient. Phase 4 needs both 2
and 3 to be far enough along that the example reproduces.

A natural single-track ordering: 0 → 1 → 2 → 3 → 4. A parallel-track
ordering: 0 → 1 → (2 ‖ 3) → 4.

## Anchor tests (write before any extractor)

These exist to keep implementation honest across what will be a
multi-week, possibly multi-agent build. Each one targets a specific
architectural commitment from the plan; failing the test means the
commitment slipped, not that a metric is off by one. Specify them
before writing the code they exercise — they describe what success
looks like, independent of how the code is structured.

None should read as "the test echoes the code." If a future reader
can rewrite the implementation behind any of these tests and the
test still passes for the right reasons, it's doing its job.

### 1. §13 worked example reproduces — scenario

The toy crate from spec §13 lives in `examples/single-file-lib/`.
The test runs `cstat extract` on it and snapshots the JSON. The
snapshot is reviewed on diff, not regenerated automatically.

**Anchors:** the spec's own example is the canonical description of
expected behavior. The snapshot covers output shape, key naming, the
AST↔Symbol identity join (§13 shows both IDs on the `classify`
entity), and per-level coverage in one artifact. If §13's numbers
are wrong, the spec gets edited; if the code is wrong, the code
gets fixed. Either is fine; the snapshot makes the choice explicit.

**Does not assert:** extractor internals or call counts.

### 2. Every required level is present — property

For any non-empty Rust project, every level listed in spec §4 as
required appears in `levels` with status in `{ok, skipped, failed}`.
No required level is absent.

**Anchors:** the biggest risk in a long build across agents is that
a level gets harder than expected and silently disappears. This
test makes a punt *visible* in the output as a `skipped` or `failed`
status with a reason. Hard-to-implement levels can't quietly vanish.

**Does not assert:** that the level's metrics are correct, only that
it reported something.

### 3. Determinism — property

For any project in `examples/`, two consecutive runs produce byte-
identical JSON for every key whose owning level reports
`deterministic: true`.

**Anchors:** trajectories in Milestone 6 are built from these
vectors. If extract is non-deterministic for keys we claim are
deterministic, downstream data is noise. Hash-map iteration order,
embedded timestamps, parallel-scheduling effects all surface
immediately.

**Does not assert:** anything about non-deterministic levels, which
are inspected separately.

### 4. Schema roundtrip — property

`OracleOutput → JSON → serde_json::Value → JSON → OracleOutput` is
lossless. Generated outputs validate against a JSON schema derived
from §7.

**Anchors:** the JSON shape is the contract Milestone 2+ read.
Schema drift is invisible until the next milestone breaks. This
test surfaces drift the moment it's introduced.

**Does not assert:** values, only structural integrity.

### 5. No two levels claim the same key — property

Across every registered level, the set of vector keys emitted is
pairwise disjoint between levels. Provenance for each key names
exactly one owning level.

**Anchors:** the namespace stays meaningful (`ast.*` is AST, `mir.*`
is MIR, `sym.*` is symbol table). Two levels silently double-writing
the same key would mean last-writer-wins and half the data lost
without notice. Phase 1's orchestrator cross-checks claim vs.
emission; this test makes that check observable.

**Does not assert:** that the partitioning is the right
partitioning. That's a design decision baked into the spec.

### 6. Unparseable file — contract (§11 row 4)

Fixture: a project with one well-formed `.rs` file and one
syntactically broken one. Run extract.

Assert: the broken file appears as a record under
`entities.ast.file_errors`; AST-level metrics for the good file are
present; `meta.files_unparseable.count == 1.0`; the process exits
zero.

**Anchors:** it's tempting to propagate a `syn` parse error upward
and abort. The spec says no — file-level failures degrade
gracefully. This makes that rule enforceable.

**Does not assert:** the reason string verbatim, only that the
record exists and the meta counter increments.

### 7. Nightly unavailable — contract (§11 "tool missing")

Fixture: any project, run with an env override that simulates
missing nightly (or actually unset, in a CI lane without nightly).

Assert: every level in §2.3 (rustc-internal IRs), level 9, level
18's nightly path, and the rustdoc-JSON-derived levels (17, 19, 46)
report status in `{skipped, failed}` with a reason. Every stable-
Rust level reports `ok`. Process exits zero.

**Anchors:** the architectural commitment that the stable
orchestrator works without nightly is a runtime requirement, not
just a build-time concern. Missing nightly produces a usable
vector, not a crash.

**Does not assert:** the reason string verbatim, only that status
is one of the documented values and a reason field is non-empty.

### 8. Isolation boundary — architectural check

A CI step runs `cargo tree -p cstat-orchestrate` (and the other
stable crates) and asserts that no `rustc_*` crate appears anywhere
in the transitive dependency graph. The only crate permitted to
depend on `rustc_*` is `cstat-extract-rustc`.

**Anchors:** the load-bearing decision of the whole plan. If
`rustc_driver` sneaks into the stable side once, undoing it later
means recompiling on every rustc bump forever. This catches the
slip on the first PR that introduces it.

**Does not assert:** anything about behavior. It's a structural
invariant on the build graph.

---

Tests 1, 6, and 7 use checked-in fixtures (one toy crate, one
broken-file crate, one nominal crate for the nightly-unavailable
case). Tests 2, 3, 4, 5 are universally quantified — they run
against every fixture and every example project we check in. Test 8
is a CI script, not a Rust test.

The full set should be specifiable in a single afternoon before any
extractor lands. Treat them as the contract Phase 1 must satisfy
the shape of, and Phases 2–4 must satisfy the substance of.

---

## What "done with Milestone 1" looks like

- Spec is internally consistent (Phase 0).
- Workspace builds on stable; `cstat-extract-rustc` builds on its
  pinned nightly.
- `cstat extract --path <any-rust-project>` produces a JSON matching
  §7 with every required level reporting `status: "ok"` when the
  prerequisites are present.
- The §13 worked example reproduces (modulo spec corrections to the
  numbers).
- Validation tests pass.
- No moves, no optimizer, no scoring — just the coordinate system,
  fully observable.
