# cstat-driven codebase cleanup — agent reference

## What is cstat

cstat is a static analysis CLI for Rust codebases. It measures structure; it
does not modify code. Use it before and after a cleanup stage to verify that
objective metrics moved in the intended direction while the project's own build
and tests preserve behavior.

## Starting point

Start with the focused commands that answer the current cleanup question:

```sh
cstat scorecard --json --path <project_root>
cstat loc --json --path <project_root>
cstat symbols --json --path <project_root>
cstat deps --json --path <project_root>
cstat dead-code --json --path <project_root>
cstat test-reachability --json --path <project_root>
```

The stale `summary`, `report`, and `advanced` entry points have been removed.
Use focused root commands directly.

## Focused drilldowns

Each accepts `--json` for structured output.

- `cstat loc --explain --json` — machine-readable `loc` contract: modes,
  `code_lines` rules, project JSON fields, and selected-file JSON fields.
- `cstat loc --json --path .` — project size-shape data.
- `cstat loc --json --path src/lib.rs` — selected-file projected static line
  reachability.
- `cstat symbols --json --path .` — symbol totals by kind and per file; pass a
  Rust source file to `--path` for selected-file rows and line spans.
- `cstat deps --json --path .` — dependency edges, coupling, fan-in/fan-out,
  and cohesion.
- `cstat dead-code --json --path .` — static cold-function candidates.
- `cstat test-reachability --json --path .` — static test/benchmark
  reachability and reaching-entry counts.
- `cstat branching --json --path .` — per-function decision/path complexity.
- `cstat signature --json --path .` — per-function API boundary complexity.
- `cstat span --json --path .` — per-function implementation span metrics.
- `cstat scorecard --json --path .` — deterministic structural complexity cost
  for agent optimization, with component costs, scope breakdown, and hotspots.


## Cleanup workflow

1. Run the focused `cstat` commands that match the cleanup target.
2. Read the focused sections in this order:
   - line counts: find oversized files first;
   - symbols: find files with too many definitions;
   - dependency matrix: find high fan-in/fan-out modules and cycles;
   - dead code: review cold candidates before deleting;
   - test reachability: compare code surface against what tests/benches can statically reach.
3. Plan one cleanup stage at a time.
4. After each stage, rerun the same focused `cstat` commands and the project's
   own build/tests. cstat metrics do not prove correctness.

## Safety rules

- Dead-code findings are static. Check macros, trait-object calls, public API
  use, build scripts, and string-based dispatch before deleting.
- Test-reachability counts are static entry-root reachability: how many
  test/benchmark roots can reach a function or edge. They are not runtime hit-count profiling.
- Dependency edges come from source-level `use`/`mod` relationships. Generated
  code and macro expansion can hide edges.
- For exact `loc` `code_lines` rules and JSON fields, run
  `cstat loc --explain` or `cstat loc --explain --json`; that command is the
  canonical contract.
- Symbol counts come from `syn` Rust AST parsing. They are not semantic name
  resolution, rustc integration, macro expansion, or proof of public API usage.
  Parse errors are reported instead of ignored; selected-file mode is used when
  `--path` points at a Rust source file under a crate's source, test, or bench
  root.
- Prefer targeted reductions: remove dead code, split large files, move symbols
  across modules, then reduce per-function complexity.
