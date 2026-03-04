 cstat-driven codebase cleanup — agent reference                           
                                                                            
  What is cstat                                                             

  cstat is a static analysis CLI for Rust codebases. It parses ASTs, builds
  call graphs and dependency graphs, computes complexity metrics, detects
  redundancy, and measures modularity. It does NOT modify code — it only
  measures. You read its output, decide what to change, then re-measure to
  confirm improvement.

  Commands

  Starting point — run this first

  cstat dump --path <project_root>

  Emits a compact JSON summary (~3-5KB) containing:
  - scores: modularity, complexity, maintainability — each 0.0–1.0, higher
  is healthier
  - diagnostic_counts: alert/warn counts per category — tells you where
  problems cluster
  - worst_items: top 5 most severe diagnostics with full context — tells you
   what is worst
  - suggested_drilldowns: ranked list of follow-up commands to run, with
  reasons — tells you what to look at next

  Drilldown commands — run these for detail

  Each accepts --json for structured output, or omit for human-readable
  terminal rendering.

  Command: cstat complexity --json --path .
  What it shows: Per-function cyclomatic, cognitive, nesting, parameter
    counts, composite scores, line ranges
  Use when: bloated_function, high_complexity, high_cognitive, deep_nesting
    diagnostics
  ────────────────────────────────────────
  Command: cstat deps --json --path .
  What it shows: Module dependency edges, coupling pairs, fan-in/fan-out,
    cohesion scores
  Use when: low_cohesion, high_coupling, god_module diagnostics
  ────────────────────────────────────────
  Command: cstat redundancy --json --path .
  What it shows: Pairs of similar/duplicate functions with similarity scores

    and types (signature/structural/both)
  Use when: redundant_code diagnostics
  ────────────────────────────────────────
  Command: cstat loc --json --path .
  What it shows: Per-file line counts, size distribution, directory
  breakdown
  Use when: bloated_file diagnostics
  ────────────────────────────────────────
  Command: cstat datapaths --json --path .
  What it shows: Function reachability from entry points, cold (unreachable)

    functions
  Use when: dead_code diagnostics
  ────────────────────────────────────────
  Command: cstat flow --json --path .
  What it shows: Call graph edges, cycles, max call depth, entry points
  Use when: Understanding call structure before refactoring
  ────────────────────────────────────────
  Command: cstat graph --json --path .
  What it shows: SCCs (cyclic clusters), betweenness centrality, PageRank,
    clustering coefficient
  Use when: cyclic_deps diagnostics, finding bottleneck modules
  ────────────────────────────────────────
  Command: cstat circuits --json --path .
  What it shows: Functional clusters via community detection, modularity Q,
    inter-cluster edges
  Use when: Understanding natural module boundaries for splitting files

  Reference — for understanding metrics

  cstat guide <topic>

  Topics: getting-started, size, complexity, modularity, flow, redundancy,
  architecture. Each explains what the metrics measure and what patterns to
  look for. Use --json for structured output.

  Cleanup workflow

  Phase 1: Assess

  1. Run cstat dump --path . — read scores and diagnostic counts
  2. Run each command listed in suggested_drilldowns
  3. Catalog the problems by category. Understand the landscape before
  changing anything.

  Phase 2: Plan stages (for your plan.md)

  Organize cleanup into stages that can each be verified independently.
  Recommended ordering — each stage makes subsequent stages safer and
  easier:

  Dead code first → removing unused code can't break callers, and it reduces
   noise in every subsequent analysis.

  Redundancy consolidation second → merging duplicate functions reduces the
  surface area you need to reason about for the remaining stages. Identify
  which copy is canonical (most callers, best tested), consolidate to it,
  update call sites.

  Dependency detangling third → now that dead and duplicate code is gone,
  the dependency graph is cleaner. Break bidirectional coupling, reduce god
  modules' fan-in/fan-out, improve cohesion by moving misplaced functions to
   the modules they actually belong with.

  File splitting fourth → for bloated files, use cstat circuits --json to
  find natural cluster boundaries within a file. Split along those
  boundaries. Functions in the same cluster call each other frequently —
  keep them together.

  Complexity reduction last → extract helper functions from deeply nested or
   high-complexity functions. This is the most likely to introduce bugs, so
  do it after the structure is clean.

  Phase 3: Verify each stage

  After each stage, re-run cstat dump --path . and confirm:
  - Scores did not decrease (or decreased only trivially)
  - The diagnostic count for the targeted category went down
  - No new alert-level diagnostics appeared in other categories

  Also: build and test after every stage. cargo check at minimum, cargo test
   if tests exist. cstat measures structure, not correctness — passing cstat
   metrics with broken tests is meaningless.

  Diagnostic categories reference

  Category: bloated_file
  What it means: File has too many lines
  Typical fix: Split along cluster boundaries (use circuits)
  ────────────────────────────────────────
  Category: bloated_function
  What it means: Function body too long
  Typical fix: Extract sub-operations into focused helpers
  ────────────────────────────────────────
  Category: high_complexity
  What it means: Too many branching paths (cyclomatic)
  Typical fix: Simplify control flow, extract match arms
  ────────────────────────────────────────
  Category: high_cognitive
  What it means: Hard for a human to follow (cognitive)
  Typical fix: Reduce nesting, early returns, decompose
  ────────────────────────────────────────
  Category: deep_nesting
  What it means: Deeply nested control structures
  Typical fix: Flatten with early returns, extract inner blocks
  ────────────────────────────────────────
  Category: high_coupling
  What it means: Bidirectional module dependencies
  Typical fix: Break the cycle — one side should depend on the other, not
    both
  ────────────────────────────────────────
  Category: god_module
  What it means: Module everything depends on or that depends on everything
  Typical fix: Extract focused sub-modules
  ────────────────────────────────────────
  Category: dead_code
  What it means: Function unreachable from any entry point
  Typical fix: Remove it (verify no dynamic/macro invocation first)
  ────────────────────────────────────────
  Category: low_cohesion
  What it means: Functions in a module don't relate to each other
  Typical fix: Move unrelated functions to where they belong
  ────────────────────────────────────────
  Category: redundant_code
  What it means: Two functions with similar signatures or structure
  Typical fix: Consolidate to one, update callers
  ────────────────────────────────────────
  Category: too_many_params
  What it means: Function takes too many arguments
  Typical fix: Group related params into a struct, or decompose the function
  ────────────────────────────────────────
  Category: cyclic_deps
  What it means: Strongly connected component in call/dep graph
  Typical fix: Break the cycle by extracting shared logic

  Safety rules

  - Build after every stage. If it doesn't compile, the stage isn't done.
  - Test after every stage. If tests fail, revert and try a different
  approach.
  - Dead code removal: before deleting, grep for the function name in
  macros, build scripts, and string literals. cstat datapaths uses static
  analysis and can miss dynamic dispatch.
  - Redundancy consolidation: when merging two functions, check that their
  behavior is truly identical, not just structurally similar. Similarity
  score > 0.9 with type structural or both is high confidence.
  - File splitting: keep pub interfaces stable. Moving a function to a new
  module may require re-exporting it from the original module to avoid
  breaking downstream use statements.
  - Don't chase perfection. Warn-level diagnostics are informational. Focus
  on alerts. A codebase with zero alerts and some warns is healthy.

  Example plan.md structure

  # Plan: Codebase cleanup via cstat analysis

  ## Stage 1: Remove dead code
  Identify unreachable functions via `cstat datapaths --json`. Remove them.
  Verify: `cstat dump` shows dead_code count at 0. Build passes.

  ## Stage 2: Consolidate redundant functions
  Identify duplicate/similar function pairs via `cstat redundancy --json`.
  For each alert-level pair, consolidate to the canonical version and update
   callers.
  Verify: `cstat dump` shows redundant_code alerts reduced. All tests pass.

  ## Stage 3: Detangle module dependencies
  Identify bidirectional coupling and god modules via `cstat deps --json`.
  Break cycles by extracting shared interfaces or inverting dependencies.
  Verify: `cstat dump` shows coupling/god_module alerts reduced. Cohesion
  scores stable or improved.

  ## Stage 4: Split bloated files
  For each bloated_file alert, use `cstat circuits --json` to find cluster
  boundaries.
  Split into focused modules along those boundaries. Re-export public items
  as needed.
  Verify: `cstat dump` shows bloated_file count reduced. No new low_cohesion
   alerts.

  ## Stage 5: Reduce function complexity
  Address alert-level bloated_function and high_cognitive items from `cstat
  complexity --json`.
  Extract helpers, flatten nesting, simplify control flow.
  Verify: `cstat dump` shows complexity score improved. All tests pass.

  Adapt the stages to what cstat dump actually reports for the target
  project. If a category has zero alerts, skip that stage. If one category
  dominates, it might warrant multiple stages.


