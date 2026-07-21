# cstat-driven codebase cleanup — agent reference

## What is cstat

cstat is a static analysis CLI for Rust codebases. It measures structure; it
does not modify code. Use it before and after a cleanup stage to verify that
objective metrics moved in the intended direction while the project's own build
and tests preserve behavior.

## Starting point

Run the focused report first:

```sh
cstat report --json --path <project_root>
```

Omitting the command is equivalent to `cstat summary`, which is the same focused
report. The JSON contains:

- `line_counts`: total code lines, physical lines, and per-file counts.
- `symbols`: total and per-file counts for functions, structs, enums, traits,
  `trait_impls`, consts, statics, and parse-error state. `trait_impls` means
  `impl Trait for Type` blocks only, not inherent `impl Type` blocks.
- `dependencies`: module list, module interdependency matrix, edge count, and
  fan-in/fan-out degrees.
- `dead_code`: functions not statically reachable from main, tests, or
  benchmarks.
- `test_reachability`: static test/benchmark reachability, step counts, max
  depth, and per-function/per-edge reaching-entry counts.

The focused report intentionally excludes older exploratory dashboards. They
remain available under `cstat advanced ...`.

## Focused drilldowns

Each accepts `--json` for structured output.

- `cstat loc --json --path .` — line counts and directory breakdown.
- `cstat symbols --json --path .` — symbol totals by kind and per file; pass a
  Rust source file to `--path` for selected-file rows and line spans.
- `cstat deps --json --path .` — dependency edges, coupling, fan-in/fan-out,
  and cohesion.
- `cstat dead-code --json --path .` — static cold-function candidates.
- `cstat test-reachability --json --path .` — static test/benchmark
  reachability and reaching-entry counts.
- `cstat complexity --json --path .` — per-function cyclomatic, cognitive,
  nesting, parameter, and line metrics.

## Advanced / legacy commands

Use these only when the focused report points to a question they answer:

- `cstat advanced summary --json --path .` — previous broad dashboard.
- `cstat advanced flow --json --path .` — raw static call graph.
- `cstat advanced graph --json --path .` — graph centrality, SCCs, PageRank.
- `cstat advanced flow-heatmap --json --path .` — previous random-walk heatmap.
- `cstat advanced circuits --json --path .` — call-graph community detection.
- `cstat advanced redundancy --json --path .` — similar/duplicate function
  candidates.
- `cstat advanced dist --json --path .` — metric histograms/correlation views.
- `cstat advanced map --json --path .` — terminal architecture map.
- `cstat advanced dump --path .` — previous diagnostic-score dump.
- `cstat advanced guide <topic>` — metric reference material.

## Cleanup workflow

1. Run `cstat report --json --path .`.
2. Read the focused sections in this order:
   - line counts: find oversized files first;
   - symbols: find files with too many definitions;
   - dependency matrix: find high fan-in/fan-out modules and cycles;
   - dead code: review cold candidates before deleting;
   - test reachability: compare code surface against what tests/benches can statically reach.
3. Plan one cleanup stage at a time.
4. After each stage, rerun `cstat report --json --path .` and the project's own
   build/tests. cstat metrics do not prove correctness.

## Safety rules

- Dead-code findings are static. Check macros, trait-object calls, public API
  use, build scripts, and string-based dispatch before deleting.
- Test-reachability counts are static entry-root reachability: how many
  test/benchmark roots can reach a function or edge. They are not runtime hit-count profiling.
- Dependency edges come from source-level `use`/`mod` relationships. Generated
  code and macro expansion can hide edges.
- Symbol counts come from `syn` Rust AST parsing. They are not semantic name
  resolution, rustc integration, macro expansion, or proof of public API usage.
  Parse errors are reported instead of ignored; selected-file mode is used when
  `--path` points at a Rust source file under a crate's source, test, or bench
  root.
- Prefer targeted reductions: remove dead code, split large files, move symbols
  across modules, then reduce per-function complexity.
