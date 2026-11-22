# C++ support plan

## Goal

Add useful, deterministic, source-level C++ complexity analysis to the existing
`cstat` CLI without requiring a successful build or changing existing Rust results.
The primary use case is before/after comparisons within one C++ repository.

Keep one repository and one executable. Use Tree-sitter C++ for syntax analysis;
retain `syn` for Rust. Do not introduce a plugin system or a universal AST.

## First release boundary

Support C++ for:

- `scorecard`, including `--changed [BASE]`, `--top`, human output, and JSON;
- `loc`, `symbols`, `branching`, `signature`, and `span`;
- both directory and selected-file inputs.

C++ `loc` selected-file mode reports physical source counts, not Rust's static
reachable/unreachable line projection. Its language-specific contract must be
explicit in help and JSON.

Do not port call traces, test reachability, dynamic coverage, value clustering,
or the existing `deps` command in this release. Reject those command/language
combinations explicitly. The scorecard's include graph is not a semantic call or
module-dependency graph.

## CLI and discovery contract

- Add `--language rust|cpp`, defaulting to Rust to preserve current invocations.
  Use explicit `--language cpp` for C++ files and directories in the first release.
- C++ directory roots need no Cargo, CMake, or compilation database.
- Discover `.cpp`, `.cc`, `.cxx`, `.C`, `.h`, `.hpp`, `.hh`, `.hxx`, `.ipp`,
  and `.tpp`. In explicit C++ mode, `.h` is interpreted as C++.
  C sources and C++ module units are outside the initial discovery contract.
- Honor Git ignore rules and provide repeatable root-relative `--exclude` globs
  for generated, build, and vendor trees. Report the effective selection rules;
  do not silently guess which checked-in code is third-party.
- Do not follow directory symlinks. Sort paths and count each physical source
  input once; do not multiply header contents by translation units.
- For selected files, use the enclosing Git root when available, otherwise the
  containing directory. Do not require a manifest.
- `--changed` retains current semantics: existing whole files changed against
  the base plus nonignored untracked files; skip deletions. A changed header is
  scored once, not as every dependent translation unit. This is not impact analysis.
- Mixed-language repositories produce separate language reports, not one total.

Example:

```sh
cstat scorecard --language cpp --path ../project --json
cstat scorecard --language cpp --path ../project --changed HEAD~1
cstat branching --language cpp --path ../project/src/widget.cpp
```

## Analysis semantics

### Syntax, not compilation

Parse source as written. Do not expand macros, instantiate templates, evaluate
preprocessor conditions, or resolve overloads. Report these limitations in the
analysis metadata. Analyze syntax in conditional-compilation branches where the
grammar permits; the result does not describe one active build configuration.
Macro-generated declarations and functions are not expanded or inferred.

Use distinct source identities: relative file, source span, and qualified display
name. Overloads must not collapse into a name-keyed map.

Count function bodies only for implementation complexity. Count declarations as
declaration surface, not additional implementations. Treat lambdas as separate
callables and exclude their bodies from the enclosing callable's branch,
nesting, and statement totals. Count template definitions once; explicit
specialization definitions remain separate source bodies.

Record classes, structs, enums, aliases, namespaces, templates, constructors,
destructors, operators, methods, and free functions as C++ concepts. Never map
classes to Rust traits or invent `pub(crate)` equivalents.

### Score contract

Publish `cpp_code_complexity_cost_v1`, separate from the Rust score version.
Reuse arithmetic and presentation where meanings actually match, not merely
because existing structs have convenient fields.

Before implementing the scalar, specify and fixture-test:

- branch rules for conditionals, loops, switch alternatives, catches, ternaries,
  and short-circuit operators;
- nesting, statement, parameter, template, constraint, and type-complexity rules;
- file concentration and named declaration surface;
- syntactic accessibility/header surface, explicitly not ABI/export visibility;
- include-coupling weights, with no fabricated call-based cohesion metric;
- source-size policy: physical LOC is descriptive; use structural counts for
  scoring so packing statements onto one line cannot improve the scalar.

Do not copy Rust-specific trait, visibility, or `include!` shard penalties.
Keep any C++ surface cost local to the component that owns it so declaration
and definition accounting does not accidentally duplicate implementation cost.

For include coupling, resolve literal quoted includes relative to the source
file and then configured include roots; resolve angle includes against configured
roots. Provide repeatable `--include-dir` for this purpose only, not preprocessing.
Report unresolved, external, and macro includes separately; never guess using
basename matches. Use a directed file graph with unique edges, fan-in/fan-out,
and cycles, not the current Rust cohesion formula.

Label test/benchmark scopes by documented directory conventions; all scopes
contribute equally. Do not claim framework-level test discovery.

### Trust and failure behavior

- C++ JSON identifies language, analysis mode, score version, file counts,
  exclusions, parser diagnostics, and unresolved includes without Rust-only keys.
- Syntax recovery is not silent success. Detect Tree-sitter error/missing nodes
  and report file/line diagnostics.
- Unreadable or unparseable selected inputs make the run incomplete and exit
  nonzero. A scorecard must not emit a normal comparable scalar for an incomplete
  run. Other reports may expose explicitly partial observations.
- Valid but macro-dependent syntax remains source-level analysis; successful
  parsing is not evidence that all compiled code was observed.
- Existing Rust JSON, score version, defaults, and error behavior remain unchanged.

## Implementation sequence

### 1. Establish the contract and parser feasibility

Build a small representative C++ fixture corpus: header/source pairs, overloads,
classes, templates, specializations, lambdas, constructors, operators, raw strings,
preprocessor alternatives, declaration macros, malformed input, and include cycles.
Check the pinned Tree-sitter runtime/grammar pair against it. Finalize the C++
score rules and JSON examples before wiring the scalar.

Gate: known extraction boundaries and deterministic expected metrics; no promises
of semantic resolution. If representative macro-heavy code cannot be parsed
usefully, revisit the backend decision before implementing the scorer.

### 2. Add routing and discovery

Modify `src/main.rs` and `src/discovery.rs` for explicit language dispatch,
C++ file selection, exclusions, include roots, and changed-file filtering.
Keep the existing Rust discovery path intact. Reject unsupported C++ commands.

Gate: ordinary directories, header-only projects, selected files, mixed repos,
and Git changed-file selection behave according to the contract.

### 3. Implement C++ extraction and basic reports

Add `src/cpp/` for parser traversal, metrics, discovery-specific helpers, and
include analysis. Parse each file once per invocation and retain metrics and
source spans, not a second general-purpose AST.

Deliver `loc`, `symbols`, `branching`, `signature`, and `span`. Share rendering
helpers where practical; retain language-specific rows where semantics differ.

Gate: fixture metrics are correct, overloads survive, lambda bodies are not
charged twice, and every parse failure is visible.

### 4. Integrate the C++ scorecard

Extract only the small arithmetic/report helpers genuinely used by both paths
from `src/scorecard.rs` and related modules. Keep C++ analysis and formula inputs
independent of `ast_parser::ProjectSymbols`, `flow::CallGraph`, and Rust visibility.
Do not rewrite the Rust backend as part of this work.

Deliver complete C++ components, deterministic contributor ordering, scope
breakdowns, `--top`, and `--changed`. Maintain separate language-specific JSON
adapters where needed to preserve the Rust schema without Rust-only C++ fields.

Gate: an end-to-end C++ scorecard with explainable contributors and no missing
components disguised as zero-valued Rust metrics.

### 5. Validate and document

Add targeted regression tests for risky consumer-visible behavior:

- declaration/definition and lambda accounting;
- branch/signature extraction and overloaded identities;
- shared headers, include resolution/cycles, and unresolved include reporting;
- parse failures and incomplete-score handling;
- ignored/excluded files and changed-header selection;
- formatting-only changes not reducing the structural score;
- equal results across repeated runs and contributor limits not altering totals.

Run focused C++ tests and the existing Rust CLI contract tests after integration.
Launch actual human and JSON CLI commands against fixtures and at least one
representative real C++ project; inspect parser diagnostics and top contributors.
Measure elapsed time and peak memory on that project. Do not claim broad C++
compatibility from fixtures alone.

Update README, command help, and the score specification with language boundaries,
examples, failure behavior, and interpretation of the C++ score.

## Later, separate decision

Resolved call traces and build-aware reachability require a Clang-based backend
and `compile_commands.json`. Coverage additionally needs an explicit build/test
runner contract. These are separate projects after the syntax-only release proves
useful; do not prebuild an abstraction for them now.

## Estimate

Planning estimate, not a measured implementation result: roughly 1–2 engineer
weeks for the defined first release, with 2–3 weeks a safer allowance for macro
compatibility, score calibration, and real-project validation. Re-estimate after
the parser feasibility gate. Full CLI parity is not included.
