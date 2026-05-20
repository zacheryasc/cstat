# Milestone 1 Specification — Representation Stack

## 1. Scope and Non-Goals

This document specifies the representation layer of the codebase-simplifier
oracle. It defines (a) the set of representation levels the oracle can
ingest, (b) the state-vector schema produced by ingestion, and (c) the
contracts (determinism, versioning, failure modes, extensibility) that
consumers can rely on.

**In scope for v1:** every representation level listed in §2.1–§2.9.
Levels reachable on stable Rust are extracted directly. rustc-internal
IRs (HIR, THIR, MIR, borrowck artifacts, mono-items) are also in v1,
accessed via `rustc_driver` in an isolated nightly sub-binary that
translates rustc's internal data structures into our own serializable
mirror types before they cross the process boundary. The rest of the
system depends only on those mirror types and stays on stable Rust.

**Out of scope (this milestone):** dynamic profiling, runtime traces,
fuzzer-derived coverage, performance counters, git/temporal data. These
are real signals but belong to later milestones or different tools.

**Out of scope (permanently):** human judgment, taste calibration,
stylistic preference. The vector is structural; we do not measure
"readability."

---

## 2. Representation Levels

Levels are grouped from highest to lowest abstraction. Each entry lists:
what is uniquely visible there, access mechanism, extraction cost class,
determinism, and v1 inclusion.

Cost classes: **F** = free during a build we already run; **L** = a
local pass over already-extracted artifacts; **B** = requires a build or
rebuild; **C** = requires a codegen step beyond a normal check build.

### 2.1 Textual and Pre-Syntactic Levels

| # | Level | Uniquely visible | Access | Cost | Deterministic | v1 |
|---|-------|------------------|--------|------|---------------|----|
| 1 | Filesystem layout | Directory tree, file sizes, file count per dir, naming patterns | `walkdir` | F | yes | yes |
| 2 | Raw bytes | Encoding, BOM, line endings, file size in bytes | direct read | F | yes | yes |
| 3 | Source text | Char-level content, line-level structure, blank-line density | direct read | F | yes | yes |
| 4 | Token stream | Lexical density, identifier vocabulary, raw vs cooked strings, operator frequency | `proc_macro2::TokenStream` / `syn` lex | F | yes | yes |
| 5 | Comment stream | Doc-comment vs line-comment ratio, comment density, TODO/FIXME markers, comment-to-code ratio | token-stream filter | F | yes | yes |
| 6 | Whitespace and formatting | Indentation style, line-length distribution, rustfmt-conformance | rustfmt --check / token-span analysis | F | yes | yes |

### 2.2 Syntactic Levels

| # | Level | Uniquely visible | Access | Cost | Deterministic | v1 |
|---|-------|------------------|--------|------|---------------|----|
| 7 | Concrete syntax tree (CST) | Exact written structure including unexpanded macro calls, attribute placement | `syn::parse_file` | L | yes | yes |
| 8 | Item tree / module skeleton | Module hierarchy, declared items per module, visibility distribution | CST walk | L | yes | yes |
| 9 | Macro-expanded AST | Code after `macro_rules!` and proc-macro expansion; reveals real branching/operator counts | `cargo expand` (subcommand) or rustc `-Zunpretty=expanded` (nightly) | B | mostly (proc macros may read env/time) | yes (best-effort; degrade if proc-macros non-deterministic) |
| 10 | Name-resolved AST | Each identifier bound to its definition; import graph; shadowing structure | rust-analyzer crate API or `rustc_resolve` | B | yes | deferred to v1.1 |

### 2.3 Compiler-Internal IRs

| # | Level | Uniquely visible | Access | Cost | Deterministic | v1 |
|---|-------|------------------|--------|------|---------------|----|
| 11 | HIR (High-level IR) | Desugared control flow with source attribution preserved | rustc_driver, in-process inside the nightly sub-binary | B | yes (per toolchain) | yes |
| 12 | THIR (Typed HIR) | HIR with full type information attached | rustc_driver, in-process inside the nightly sub-binary | B | yes (per toolchain) | yes |
| 13 | MIR (pre-opt) | Three-address control-flow graph per function, before MIR opts | rustc_driver, in-process inside the nightly sub-binary | B | yes (per toolchain) | yes |
| 14 | Optimized MIR | MIR after const-prop, inlining, dead-code passes | rustc_driver, in-process inside the nightly sub-binary | B | yes (per toolchain) | yes |
| 15 | Borrowck artifacts | Region/lifetime constraints, ownership transfer graph | rustc_driver, in-process inside the nightly sub-binary | B | yes (per toolchain) | yes |

Ingestion happens out-of-process. The nightly sub-binary uses
`rustc_driver` to walk these IRs and translates them into stable
mirror types (a separate crate, `cstat-ir`) before emitting JSON to
the orchestrator. The orchestrator never sees a `TyCtxt` or any
rustc-internal type. When rustc's internal data structures change,
exactly one translator updates; the rest of the system is unaffected.

### 2.4 Type-System Derived Views

| # | Level | Uniquely visible | Access | Cost | Deterministic | v1 |
|---|-------|------------------|--------|------|---------------|----|
| 16 | Type usage graph | Which types flow through which functions, per-fn distinct type count | AST (approximate) + nightly sub-binary (exact, name-resolved) | L–B | yes | yes |
| 17 | Trait impl graph | Trait → impls → types, orphan rules, blanket impls | rustdoc JSON (nightly) | B | yes (per toolchain) | yes |
| 18 | Generic instantiation map | Which generic items get monomorphized at which type substitutions | `tcx.collect_and_partition_mono_items` via the nightly sub-binary | B | yes (per toolchain) | yes |
| 19 | Coherence/orphan structure | Distribution of impls across crates, downstream-impl risk | rustdoc JSON | B | yes (per toolchain) | yes |

### 2.5 Codegen Levels

| # | Level | Uniquely visible | Access | Cost | Deterministic | v1 |
|---|-------|------------------|--------|------|---------------|----|
| 20 | LLVM IR (pre-opt, post-mono) | All monomorphized instances, IR-level operations per function | `cargo rustc -- --emit=llvm-ir -C no-prepopulate-passes` | C | yes (per toolchain) | yes |
| 21 | Optimized LLVM IR | Post LLVM optimization passes; inlining decisions, dead code elimination | `cargo rustc -- --emit=llvm-ir` (default opt level) | C | yes (per toolchain + opt level) | yes |
| 22 | Assembly | Per-target instruction mix, function sizes in instructions | `cargo rustc -- --emit=asm` | C | yes (per target) | yes |
| 23 | Object files | Per-object code/data section sizes, relocation count | build output | C | yes | yes |
| 24 | Linked binary | Total binary size, section layout, deduplication wins | build output | C | yes | yes |

### 2.6 Binary-Artifact Levels

| # | Level | Uniquely visible | Access | Cost | Deterministic | v1 |
|---|-------|------------------|--------|------|---------------|----|
| 25 | Symbol table | Total symbol count, source-fn → symbol ratio (monomorph pressure), symbol size distribution | `nm` / `object` crate | C | yes | yes |
| 26 | Demangled symbol graph | Generic instantiations recoverable by demangling | `rustc-demangle` over §2.6 #25 | L | yes | yes |
| 27 | Section layout | .text / .rodata / .data / .bss sizes, alignment waste | `object` crate | C | yes | yes |
| 28 | Relocation info | Number/kind of relocations, indirect-call density | `object` crate | C | yes | yes |
| 29 | DWARF debug info | Source-to-symbol mapping, inline call sites, generic instantiation provenance | `gimli` crate over debug build | C | yes | yes |
| 30 | Linker dependency graph | Which compilation units pull in which symbols; dead-strip residue | linker map file | C | yes | yes |

### 2.7 Graph-Derived Representations

Computed from one or more of the above. Listed separately because they
are the structures the optimizer most directly reasons over.

| # | Level | Source levels | Cost | v1 |
|---|-------|---------------|------|----|
| 31 | Module dependency graph | §2.2 #8 + use-resolution | L | yes (cstat has this) |
| 32 | Intra-crate call graph (static) | §2.2 #7–9 | L | yes (cstat has this) |
| 33 | Inter-crate / monomorph call graph | §2.6 #26, §2.5 #20 | L | yes (LLVM-derived) |
| 34 | Control-flow graph per function | §2.2 #7 or §2.5 #20 | L | yes (AST-CFG in v1) |
| 35 | Data-flow graph per function | §2.5 #20 (def-use chains in LLVM) | L | yes (LLVM-derived) |
| 36 | Type-usage graph | §2.4 #16 | L | yes |
| 37 | Strongly-connected components / cycles | §2.7 #31, #32 | L | yes (cstat has this) |
| 38 | Community/cluster decomposition | §2.7 #31, #32 | L | yes (cstat has this) |
| 39 | Power-law fit of degree distributions | any graph in §2.7 | L | yes |
| 40 | Self-similarity / cross-scale shape comparison | per-fn vs per-file vs per-module distributions of any scalar | L | yes |

### 2.8 Project-Level Metadata

| # | Level | Uniquely visible | Access | Cost | v1 |
|---|-------|------------------|--------|------|----|
| 41 | Cargo workspace metadata | Crate list, dependency declarations, features, edition, MSRV | `cargo metadata --format-version 1` | F | yes |
| 42 | Cargo.lock | Resolved dep versions, dep depth, dep count | direct parse | F | yes |
| 43 | Active cfg/feature set | Which `#[cfg]` branches are live for the current build | rustc output / `--print cfg` | F | yes |
| 44 | Toolchain pin | rust-toolchain.toml content | direct read | F | yes |

### 2.9 External-Tool-Derived (Optional)

| # | Level | Source | Cost | v1 |
|---|-------|--------|------|----|
| 45 | Clippy lint inventory | `cargo clippy --message-format=json` | B | yes |
| 46 | rustdoc JSON | `cargo +nightly rustdoc -- --output-format json` | B | yes |
| 47 | rustfmt diff size | `cargo fmt -- --check` | F | yes |
| 48 | Test inventory | `cargo test -- --list --format json` (`unstable-options`) | B | yes |
| 49 | Doc-test inventory | rustdoc | B | yes |

### 2.10 Excluded From v1 (For Completeness)

| Level | Reason |
|-------|--------|
| Runtime traces / dtrace / perf | Dynamic, not structural |
| Coverage maps | Belongs to behavioral testing layer |
| Benchmark output | Performance, not structure |
| Git history | Deferred per roadmap principle #6 |
| Issue tracker / PR data | Human-derived, principle #1 |

---

## 3. Access Mechanisms and Tool Stability

| Mechanism | Stability | Notes |
|-----------|-----------|-------|
| `walkdir`, direct file I/O | stable | foundational |
| `syn`, `proc_macro2` | stable (crates.io) | current cstat dependency |
| `cargo metadata` | stable CLI | format version pinned |
| `cargo expand` | stable subcommand, calls nightly under the hood | runtime-detected; absent → level reports `skipped` |
| `cargo rustc -- --emit=...` | stable | LLVM IR / asm / obj emission |
| `object` crate | stable | symbol / section / relocation reading |
| `gimli` crate | stable | DWARF parsing |
| `rustc-demangle` | stable | symbol demangling |
| rustdoc JSON | nightly, unstable format | runtime-detected; absent → level reports `skipped` |
| `rustc_driver` (in-process) | nightly, unstable API | used only inside `cstat-extract-rustc`; output translated to stable mirror types before crossing the process boundary |

Nightly-dependent levels degrade to `status: skipped` when the nightly
toolchain is unavailable on the host. The oracle still produces a
well-formed vector covering every level whose prerequisites are
present. Missing-level semantics: §8.

---

## 4. v1 Inclusion Decisions — Summary

**Required in v1:** every level in §2.1 through §2.9. The stable
extractor covers everything reachable without rustc internals; the
nightly sub-binary covers levels 10, 11–15, and 18 (exact), with
level 16's name-resolved variant joining the AST-approximation. When
a level's prerequisites are unavailable on a given host (no nightly
installed, no `cargo clippy`, no debug symbols, etc.) that level
reports `status: skipped` or `failed` per §7 and the run continues.
A required level is never silently absent from the output.

**Excluded permanently from v1:** §2.10.

---

## 5. State Vector Schema

This section is the contract. Later milestones — move catalog (M2),
trajectory dataset (M6), optimizer (M7) — read this shape. Schema
versions per §10; existing keys do not change meaning across patch
or minor versions. Adding a metric means adding a key; removing or
renaming one is a major-version bump.

The state vector is a single flat namespace mapping dotted keys to
scalars.

**Key format:** `<level>.<entity>.<metric>[.<aggregate>]`

- `<level>` — short level identifier (e.g. `fs`, `ast`, `mir`, `llvm`,
  `sym`, `dwarf`, `cargo`, `graph.call`, `graph.mod`).
- `<entity>` — what the metric is per (`file`, `func`, `mod`, `crate`,
  `symbol`, `edge`, `scc`, `cluster`, or `global`).
- `<metric>` — the measured quantity (`loc`, `cyclomatic`, `cognitive`,
  `nesting`, `params`, `surface_in`, `surface_out`, `halstead_volume`,
  `halstead_difficulty`, `instability`, `abstractness`, `mono_count`,
  `symbol_size`, `inline_ratio`, `fan_in`, `fan_out`, `cohesion`,
  `degree_exponent`, `clustering_coef`, `pagerank`, `betweenness`, ...).
- `<aggregate>` — optional rollup (`p50`, `p75`, `p90`, `p99`, `mean`,
  `median`, `max`, `sum`, `count`, `stddev`, `skew`). Absent for
  per-entity vectors.

**Value type:** `f64`. Counts are `f64` for uniformity. Booleans become
0.0/1.0.

**Examples:**
```
ast.func.cyclomatic.p90         = 12.0
ast.func.cyclomatic.max         = 47.0
ast.func.cyclomatic.count_over_warn = 18.0
ast.mod.cohesion.mean           = 0.41
graph.call.degree_exponent      = 2.3
graph.mod.modularity_q          = 0.62
llvm.func.ir_ops.mean           = 84.0
sym.global.source_to_symbol_ratio = 3.7
sym.func.size_bytes.p99         = 4096.0
dwarf.global.inline_ratio       = 0.34
fs.global.file_count            = 41.0
cargo.global.direct_deps        = 9.0
cargo.global.transitive_deps    = 137.0
```

**Per-entity sub-vectors:** When a consumer needs per-function or
per-module detail (e.g., to plan a move), the oracle exposes it
separately under `entities.<level>.<entity_kind>`, keyed by stable
identifier (see §6). The flat vector contains only aggregates.

**No aggregation across levels at this layer.** A consumer wanting an
overall "complexity score" computes it from the flat vector; the oracle
does not bake one in.

---

## 6. Cross-Level Identity

Moves and metrics frequently need to refer to "the same function" or
"the same module" across levels. The oracle issues stable IDs:

- **FileId** — relative path from project root, posix-normalized.
- **ModuleId** — crate-qualified module path, e.g.
  `cstat::diagnostics::scoring`.
- **AstFuncId** — `(FileId, fully_qualified_path, item_kind, ast_node_hash)`
  where `ast_node_hash` is a structural hash of the item, used to
  distinguish overloads / multiple `impl` blocks and to detect when the
  item has changed across runs.
- **SymbolId** — mangled symbol name (post-rustc mangling), used at LLVM
  IR, symbol table, DWARF, object levels.
- **MonoId** — demangled symbol with type substitutions, joinable to
  `AstFuncId` via DWARF (for debug builds) or via rustc mono-items
  output (deferred).

Every per-entity record carries the set of IDs by which it is known.
Cross-level joins are explicit table joins on these IDs, not implicit.

When a join fails (e.g., LLVM symbol has no DWARF entry mapping it back
to AST), the record is retained with a null `AstFuncId` and contributes
to a `join_failure_rate` scalar in the global vector.

---

## 7. Oracle Interface Contract

**Invocation:** library API and CLI both supported.

**Input:**
- project root path
- config (which optional levels to enable, toolchain mode
  stable/nightly, build profile debug/release, target triple if
  cross-compiling)
- previous-run vector (optional; enables `cstat diff` semantics)

**Output:**
```
{
  "schema_version": "<semver>",
  "cstat_version": "<semver>",
  "toolchain": { "channel": "...", "version": "...", "host": "..." },
  "config_digest": "<hash>",
  "wall_clock_seconds": <f64>,
  "levels": {
    "<level>": {
      "status": "ok" | "skipped" | "failed",
      "reason": "...",                 // present if not ok
      "recompute_cost_class": "F"|"L"|"B"|"C",
      "deterministic": true|false,
      "extraction_seconds": <f64>
    }, ...
  },
  "vector": { "<dotted.key>": <f64>, ... },
  "entities": {
    "<level>.<entity_kind>": [ { "ids": {...}, "metrics": {...} }, ... ]
  },
  "provenance": {
    "<dotted.key>": ["<level>", ...]   // which levels contributed
  }
}
```

**Determinism guarantee:** For a fixed toolchain, fixed config, and
unchanged source tree, two runs produce byte-identical `vector` and
`entities` for all levels marked `deterministic: true`. Non-deterministic
levels (proc-macros that read env/time, optimized LLVM IR under certain
LLVM versions) are flagged and excluded from determinism contracts —
consumers may choose to drop them when exact reproducibility is required.

**Level failures are observable, not fatal.** Each level reports its
own status (`ok`, `skipped`, `failed`) with a reason. Level-level
failures do not fail the run; the output reflects what was
extractable, with missing-level semantics per §8. The CLI exits
non-zero only if the orchestrator itself cannot proceed: no readable
Cargo project at the path, I/O failure on the output file, or a
schema-version contract violation.

---

## 8. Missing-Level Semantics

When a level cannot be extracted:

- Its `status` in the output is `skipped` or `failed` with a `reason`.
- All vector keys that would have come from that level are **omitted**
  from `vector` (not NaN, not zero — absent). Consumers handle `missing
  key` as the explicit signal.
- `provenance` for any composite key that depended on the missing level
  is updated to reflect partial contribution.
- A scalar `meta.levels_missing.count` is always present in the vector,
  enabling consumers to detect degraded runs without inspecting every
  key.

Rationale for omission over sentinel: a sentinel like NaN propagates
through aggregations and corrupts downstream comparisons; omission forces
explicit handling at the point of use.

---

## 9. Determinism and Pinning

**Deterministic by construction:** levels 1–8, 16 (AST-approximated
form), 31–44, 47.

**Deterministic given a pinned toolchain:** levels 10–15, 16 (name-
resolved form), 17–30, 45, 46, 48, 49. This covers everything sourced
from rustc, LLVM, the linker, rustdoc, clippy, and `cargo test --list`.

**Conditionally non-deterministic:** level 9 if proc-macros read
environment or wall-clock state during expansion.

**Pinning strategy:**
- The oracle reads `rust-toolchain.toml` if present and records the
  resolved channel/version in the output.
- The oracle refuses to produce a "deterministic" stamp unless either
  (a) `rust-toolchain.toml` exists and pins a specific version, or (b)
  the caller passes an explicit `--toolchain` override.
- LLVM optimization-pass nondeterminism (rare but possible on some
  versions) is documented; level 21 is marked deterministic per
  toolchain but consumers expecting bit-exact reproducibility across
  machines must additionally pin LLVM via the toolchain.

---

## 10. Schema Versioning

`schema_version` is semver.

- **Patch (x.y.Z):** new optional vector keys added, no key removed or
  renamed, no semantic change to existing keys.
- **Minor (x.Y.0):** new required vector keys added, new levels
  added. Old consumers that ignore unknown keys remain compatible.
- **Major (X.0.0):** any key removed, renamed, redefined, or any change
  to entity-ID format. Requires explicit migration.

Trajectory data (Milestone 6) must include the schema_version that
produced it. A migration step is required to compare trajectories across
major versions; minor and patch differences are absorbed by ignoring
unknown keys / treating missing keys as omitted (§10).

The `cstat-diff` tool refuses to diff vectors across major-version
boundaries without an `--allow-migration` flag.

---

## 11. Failure Mode Catalog

| Failure | Detection | Behavior |
|---------|-----------|----------|
| Build fails (level B/C cannot proceed) | non-zero `cargo` exit | required level: oracle exits non-zero, partial vector with `meta.build_failed=1`; optional level: skipped, run continues |
| Tool missing (e.g., `nm`, `gimli` fails to parse) | tool exit / parse error | level → `failed`, reason recorded, scalars omitted |
| Timeout on level extraction | per-level timeout exceeded | level → `failed`, reason `"timeout"`, scalars omitted |
| Malformed input (un-parsable Rust) | `syn` parse error | per-file `failed` record under `entities.ast.file_errors`, file excluded from AST-level metrics, `meta.files_unparseable.count` incremented |
| Proc-macro panic during expansion | `cargo expand` non-zero | level 9 → `failed`, AST falls back to unexpanded view (level 7) |
| Mismatched toolchain (configured pin not installed) | rustup error | oracle exits non-zero before any extraction |
| Cache corruption | hash mismatch on read | cache entry discarded, level re-extracted, warning logged |
| Cross-level join failure (SymbolId has no AstFuncId) | join produces null | record retained, `join_failure_rate` updated, no run failure |

---

## 12. Extension Points

**Adding a metric to an existing level:** Implement a pass module that
takes the level's extracted data and emits `(vector_key, value)` pairs
and/or `(entity_id, metric_name, value)` tuples. Register it with the
level's pass registry. No schema change beyond patch version.

**Adding a new level:** Implement the level interface —

```
trait Level {
  fn id(&self) -> &'static str;
  fn cost_class(&self) -> CostClass;
  fn deterministic(&self) -> Determinism;
  fn extract(&self, ctx: &OracleContext) -> Result<LevelOutput>;
}
```

`LevelOutput` carries per-entity records (with IDs per §6) and any
global scalars. The orchestrator handles aggregation, provenance, cache,
and timeout. New levels require at minor version.

**Adding a move kind:** Belongs to Milestone 2's catalog. The
representation spec does not gate that work.

**Adding a representation level not anticipated above (e.g., dynamic
trace):** Same level interface. Major version bump.

---

## 13. Worked Example

Target: a single-file crate, `example/src/lib.rs`:

```rust
pub fn add(a: i32, b: i32) -> i32 { a + b }

pub fn classify(x: i32) -> &'static str {
    if x < 0 {
        "neg"
    } else if x == 0 {
        "zero"
    } else {
        "pos"
    }
}
```

Selected v1 vector output:

```
fs.global.file_count                = 1.0
fs.global.total_bytes               = 184.0
ast.global.func_count               = 2.0
ast.func.loc.p50                    = 5.0
ast.func.loc.max                    = 7.0
ast.func.cyclomatic.max             = 3.0
ast.func.cyclomatic.mean            = 2.0
ast.func.cognitive.max              = 2.0
ast.func.nesting.max                = 1.0
ast.func.params.max                 = 2.0
ast.func.params.mean                = 1.5
ast.func.surface_in.mean            = 1.0   // distinct param types
ast.func.surface_out.mean           = 1.0   // distinct return types
ast.func.halstead_volume.mean       = 38.2
tok.global.identifier_count         = 14.0
tok.global.unique_identifiers       = 9.0
tok.global.operator_density         = 0.18
graph.call.edge_count               = 0.0
graph.mod.modularity_q              = omitted     // single module, per §8
sym.global.source_to_symbol_ratio   = 1.0   // no generics
sym.func.size_bytes.max             = 64.0
sym.func.size_bytes.mean            = 48.0
dwarf.global.inline_ratio           = 0.0
cargo.global.direct_deps            = 0.0
cargo.global.transitive_deps        = 0.0
meta.levels_missing.count           = 0.0
meta.files_unparseable.count        = 0.0
```

Per-entity (excerpt):

```
entities.ast.func = [
  { ids: { AstFuncId: "lib.rs::add#h1" },
    metrics: { loc: 1, cyclomatic: 1, cognitive: 0, params: 2, ... } },
  { ids: { AstFuncId: "lib.rs::classify#h2", SymbolId: "_ZN7example8classify17h..." },
    metrics: { loc: 7, cyclomatic: 3, cognitive: 2, params: 1, ... } }
]
```

This example shows: scalars present per level, single-module Q omitted
per §10, source-to-symbol = 1 indicating no monomorphization, ID join
between AST and symbol table successful.

---

## Status

Spec drafted. Implementation plan lives in `milestone-1-dev-plan.md`.
