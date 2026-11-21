# cstat code complexity score v0

## Status

Implemented by the `scorecard` CLI command and `src/scorecard.rs`.

This score is intentionally heuristic. It is the first deterministic target for
agents to optimize before the project has behavioral data from agent cleanup
loops. Incompatible formula or JSON changes must bump `score_version`.

## Purpose

`code_complexity_cost_v2` measures structural reasoning burden in a Rust
codebase.

Lower cost means the code is structurally cleaner according to this metric.
The score is designed for agent optimization: an agent can compare before/after
runs and try to reduce the scalar by simplifying functions, reducing oversized
files, lowering coupling, shrinking excessive abstraction surface, and removing
stale code.

The score does not prove correctness, safety, maintainability, public API
fitness, or behavior preservation. Those are harness responsibilities.

## Non-goals

The score must not include:

- test pass/fail status;
- benchmark pass/fail status;
- coverage pass/fail status;
- reward shaping;
- task-specific acceptance criteria;
- model training logic;
- patch generation;
- semantic proof of dead-code safety;
- rustc-level name resolution or macro expansion requirements for v0.

The harness may reject an attempted optimization if tests, benchmarks, coverage,
protected files, or behavior checks fail. That rejection is separate from the
complexity score.

## Optimization contract

The agent-facing objective is:

```text
minimize code_complexity_cost_v2
```

The environment-facing objective is:

```text
minimize code_complexity_cost_v2 subject to harness guards passing
```

The score itself is pure structural cost. It does not contain verification
signals or guardrail penalties.

## Scope

The score covers all discovered Rust code:

- production code;
- test code;
- benchmark code;
- any future discovered Rust scope such as examples.

All scopes contribute to the default total with equal weight. Tests and benches
are code; if they are structurally complex, they increase the score. The harness
is responsible for preventing agents from deleting, weakening, or bypassing
those scopes.

The output must still include a scope breakdown so users can understand where
cost comes from:

```json
"scope_breakdown": {
  "production": { "cost": 900.0 },
  "tests": { "cost": 120.0 },
  "benches": { "cost": 30.0 }
}
```

Scope labels are explanatory. They do not change scoring weight in v0.

## Primary scalar

```text
code_complexity_cost =
    function_complexity_cost
  + file_concentration_cost
  + module_coupling_cost
  + abstraction_surface_cost
  + api_surface_cost
```

Also emit a normalized companion scalar:

```text
code_complexity_cost_per_kloc = code_complexity_cost / max(1.0, code_lines / 1000.0)
```

Use `code_complexity_cost` for before/after comparisons inside the same repo.
Use `code_complexity_cost_per_kloc` only for rough cross-repo comparisons.

## Component 1: function complexity

Function complexity is the primary local reasoning-cost signal.

For each discovered Rust function or method:

```text
function_cost =
    1.2 * cyclomatic
  + 1.5 * nesting_depth
  + 0.10 * line_count
  + 0.15 * body_stmt_count
  + 0.25 * signature_score
  + visibility_weight
  + visibility_weight * 0.5 * max(0, signature_score - 3)
```

Where:

```text
cyclomatic = branch_points + 1

signature_score =
    param_count
  + return_type_complexity
  + generic_param_count
  + trait_bound_count
  + where_predicate_count
```

Then:

```text
function_complexity_cost = sum(function_cost)
```

Rationale:

- Branching increases path reasoning.
- Nesting increases local context stack depth.
- Branching and nesting are weighted moderately so obvious gnarly functions still
  surface without dominating repository shape on their own.
- Long functions increase scan burden. Line count is also measured at file
  level, so the function-local line weight is intentionally moderate.
- Signature complexity captures call-boundary and API reasoning cost.

## Component 2: file concentration

File concentration measures how much code and symbol surface is packed into one
file.

For each discovered Rust file:

```text
file_cost =
    0.25 * code_lines
  + 3.0  * total_symbols
  + 1.0  * function_count
  + visible_non_function_symbol_surface
```

For every `include!("...")` invocation:

```text
include_shard_cost =
    200.0
  + 0.50 * included_code_lines
  + 3.00 * included_symbol_count
  + if missing_or_unparseable: 100.0 else 0.0
```

Then:

```text
file_concentration_cost = sum(file_cost) + sum(include_shard_cost)
```

Rationale:

- Large files require more navigation and context loading.
- Symbol piles make a file harder to summarize.
- Function piles usually indicate several concepts sharing one module.
- `include!` shards preserve Rust scope without creating a real module
  boundary. They should not reduce cost by hiding implementation regions in
  `.inc.rs` fragments.
- File cost uses raw counts rather than free thresholds because repository
  shape should reflect ordinary accumulated surface, not only outliers.

## Component 3: module coupling

Module coupling measures source-module navigation burden and architectural
tangling.

```text
module_coupling_cost =
    sum_per_module(
        24.0 * max(0, out_degree - 5)
      + 12.0 * max(0, in_degree - 10)
    )
  + sum_bidirectional_pairs(240.0 + 60.0 * pair_strength)
  + sum_low_cohesion_modules(
        if function_count >= 4:
            24.0 * function_count * max(0.0, 0.55 - combined_cohesion)
    )
  + sum_per_module(visible_symbol_surface)
```

Rationale:

- Fan-out means a module must know about many neighbors, so it is weighted more
  heavily than fan-in.
- Fan-in can indicate useful central code, so it receives a higher threshold and
  lower weight.
- Bidirectional pairs are strong architecture smells and receive a large fixed
  penalty.
- Low cohesion matters only when a module has enough functions for cohesion to
  be meaningful.

For v0, use existing dependency, module-degree, coupling-pair, and cohesion
signals. If full graph-cycle detection is added later, it should be a new field
or a `score_version` bump.

## Component 4: abstraction surface

Abstraction surface measures named conceptual inventory independent of line
count.

Per file or module, using symbol counts:

```text
abstraction_surface_cost =
    0.5 * max(0, structs - 12)
  + 0.5 * max(0, enums - 8)
  + 1.0 * max(0, traits - 4)
  + 0.5 * max(0, trait_impls - 8)
  + 0.2 * max(0, consts + statics - 10)
```

Then sum across files/modules.

Rationale:

- Named concepts are real reasoning surface even when individual functions are
  small.
- Traits are weighted higher because they introduce abstraction and dispatch
  reasoning.
- This component should not punish ordinary data modeling; it only penalizes
  excessive local surface.

## Component 5: API surface

API surface reports the visible symbol counts used by the other cost
components. It does not add an independent cost.

```text
api_surface_cost = 0.0
```

Visibility is folded into the local component that owns the item:

```text
visible functions       -> function_complexity_cost
visible non-functions   -> file_concentration_cost
visible module surface  -> module_coupling_cost
```

Where:

```text
visibility_weight:
  pub        = 1.0
  pub(crate) = 0.4

kind_weight:
  function/method = 1.0
  struct          = 1.0
  enum            = 1.0
  trait           = 2.0
  const/static    = 0.5
```

Rationale:

- `pub` creates repository-external compatibility surface.
- `pub(crate)` creates crate-wide non-local reasoning surface, but is cheaper
  because it remains internally changeable.
- Traits are weighted higher because they expose behavioral contracts and
  dispatch/implementation reasoning.
- Keeping visibility cost inside function, file, and module components avoids a
  separate API bucket competing with the structural buckets.

For v2, only direct `pub` and `pub(crate)` item visibility are counted. Private
items and narrower restricted visibilities such as `pub(super)` are not counted.

## JSON output contract

The score command should emit compact JSON by default when `--json` is passed:

```json
{
  "cstat_version": "0.1.0",
  "score_version": "code_complexity_cost_v2",
  "target": ".",
  "code_complexity_cost": 1234.5,
  "code_complexity_cost_per_kloc": 104.7,
  "component_costs": {
    "function_complexity": {
      "cost": 800.0,
      "functions_scored": 120
    },
    "file_concentration": {
      "cost": 120.0,
      "files_scored": 14
    },
    "module_coupling": {
      "cost": 180.0,
      "modules_scored": 14
    },
    "abstraction_surface": {
      "cost": 90.0,
      "symbols_scored": 300
    },
    "api_surface": {
      "cost": 20.0,
      "public_symbols": 12,
      "crate_symbols": 8
    }
  },
  "scope_breakdown": {
    "production": {
      "cost": 900.0,
      "functions_scored": 100,
      "files_scored": 10
    },
    "tests": {
      "cost": 300.0,
      "functions_scored": 20,
      "files_scored": 4
    },
    "benches": {
      "cost": 34.5,
      "functions_scored": 2,
      "files_scored": 1
    }
  },
  "top_contributors": [
    {
      "kind": "function",
      "scope": "production",
      "file": "src/main.rs",
      "function": "main",
      "cost": 115.4,
      "reasons": {
        "cyclomatic": 45,
        "nesting_depth": 4,
        "line_count": 212,
        "body_stmt_count": 11,
        "signature_score": 1
      },
      "component_costs": {
        "branching": 80.0,
        "nesting": 3.0,
        "span": 32.4,
        "signature": 0.0
      }
    }
  ],
  "metadata": {
    "rust_files": 25,
    "code_lines": 11793,
    "parse_error_files": 0
  }
}
```

Required stable fields:

- `cstat_version`
- `score_version`
- `target`
- `code_complexity_cost`
- `code_complexity_cost_per_kloc`
- `component_costs`
- `scope_breakdown`
- `top_contributors`
- `metadata`

Field additions are allowed within the same score version if they do not change
existing field meaning. Formula changes, field removals, or semantic changes
must bump `score_version`.

## Human output contract

Human output should be short and explanatory:

```text
cstat code complexity score
score version: code_complexity_cost_v2
lower is cleaner; harness guards behavior separately

total cost: 1234.5
cost / KLOC: 104.7

components:
  function complexity:   800.0
  file concentration:    120.0
  module coupling:       180.0
  abstraction surface:    90.0
  api surface:            20.0

top contributors:
  1. function src/main.rs::main                     115.4
  2. module src/flow                                73.0
  3. file src/coverage.rs                           47.4
```

Do not print every underlying probe. This command is a scorecard, not an `all`
command.

## Implementation guide

Recommended module:

```text
src/scorecard.rs
```

Recommended CLI:

```text
cstat scorecard --path <project-or-file> [--json] [--top N] [-v]
```

Initial implementation should support project mode first. Selected-file mode can
be added later if it naturally falls out of the data model.

Implementation steps:

1. Parse/discover Rust files using existing project discovery.
2. Parse symbols with `ast_parser::parse_project`.
3. Build function rows using the same raw fields as `branching`, `signature`,
   and `span`.
4. Count line and symbol concentration per file using existing `loc` and
   `symbols` data where possible.
5. Compute dependency/coupling/cohesion data using existing `deps` logic.
6. Compute API surface from visible parsed symbols and function signatures.
7. Classify each function/file into a scope label for reporting:
   - `production`
   - `tests`
   - `benches`
   - future labels as needed
8. Compute component costs.
9. Sum total cost.
10. Build top contributors from function, file, module, abstraction, and API
    surface contributors.
11. Render compact human output or stable JSON.

Avoid shelling out to existing CLI commands from the implementation. Reuse the
same Rust collectors directly so the score is fast, deterministic, and testable.

## Scope classification guide

Scope classification is only for explanation and breakdown in v0. It does not
change scoring weight.

Suggested rules:

- `benches`: file under `benches/` or benchmark-recognized function where known;
- `tests`: file under `tests/`, function with `#[test]`, or code inside obvious
  `#[cfg(test)]` modules where existing analysis can identify it;
- `production`: default for discovered source that is not classified above.

If classification is ambiguous, classify as `production` and avoid hiding cost.
The score should remain deterministic.

## Top contributors

Emit enough contributor detail for an agent to choose a bounded cleanup target.

Contributor kinds:

- `function`
- `file`
- `module`
- `abstraction_surface`

Each contributor should include:

- `kind`
- `scope`
- `file` or `module`
- `function` when applicable
- `cost`
- raw reason fields used to compute cost
- per-reason component costs when applicable

Default top count: 20.

## Known limitations

- The weights are guessed, not learned.
- Physical line count can be affected by formatting, so it has low weight and is
  paired with body statement count.
- Dependency extraction is source-level and can miss macro-generated edges.
- Symbol counts come from syntax parsing, not full semantic analysis.
- A lower score does not prove better architecture in every local case.
- The harness must prevent destructive or behavior-changing optimizations.

These limitations are acceptable for v0 because the score is deterministic,
explainable, and calibrated enough to start collecting before/after cleanup data.
