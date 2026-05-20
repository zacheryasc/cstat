# Roadmap: Asymptotically Optimal Codebase Simplifier

Notes May 19 26
- cstat mostly useful as code combing tool for an agent loop with
  extant verifiable behavior tests. Making a neruo-symbolic driver
  is not something well scoped enough, requires too much resources
  that you don't have. Instead just use the tool as-is, and find
  simple ways to add on to it as you find actual flaws when using it.

## Framing

The end goal is a tool that takes an existing codebase and reduces its
complexity along a vector of objective structural metrics, subject to the
constraint that observable behavior is preserved (tests pass, conformance
specs still hold). The tool should be useful with a frontier LLM acting as
the agent, and *more* useful when paired with a non-LLM optimizer — ideally
neuro-symbolic, possibly co-trained, but that architecture is a downstream
decision, not an upstream one.

cstat (this repo) is the starting point for the oracle side. It already
emits a deterministic JSON state describing modularity, complexity,
maintainability, redundancy, dead code, coupling, cohesion, and a
diagnostic list with locations. That is the seed of the state vector. The
improvements doc (`improvements-discussion.md`) expands the seed to
Halstead, instability/abstractness, information surface area, scale-free
exponents, self-similarity, and symbol/IR metrics. The current code
operates at AST level only; the future stack will be multi-level.

This document is a strategic roadmap. It defines milestones and ordering,
not implementation. Every milestone before #7 is reversible.

---

## Operating Principles

These are constraints we have already committed to. They shape every
milestone below.

1. **Objective metrics only.** No calibration against human taste. The
   target is the structural vector itself. Refactors invisible to the
   vector (renames, doc improvements, better error types) are out of scope
   for the optimizer — they live on the LLM-only frontier and are not
   measured.

2. **Multi-level representation.** The oracle must be able to consume
   information at any IR level (source, AST, HIR/MIR, LLVM IR, symbol
   table, DWARF). Moves are tagged by the level(s) at which they operate.
   Some metrics and some moves are only visible/legal at specific levels.

3. **Vector, not scalar.** Scores stay decomposed. The current
   `HealthScores` triple is a human display; the optimizer operates on the
   raw component bundle. No weighted sums imposed upstream of the
   optimizer.

4. **Pareto, not minimum.** Metrics trade against each other (collapse
   classes → fewer objects, more rigidity). The optimizer searches the
   frontier; the operator weights. Degenerate minima (delete everything,
   one God-class) are prevented by this plus the behavioral constraint.

5. **Tests are constraint, not objective.** Behavior preservation gates
   acceptance of a move. Folding correctness into the objective produces
   reward hacking.

6. **Minimize reliance on human refactor corpora inside the loop.** LLMs
   already encode that prior. Reusing it as training data confounds the
   experiment we are actually running, which is: *what does
   objective-metric-driven optimization of code structure look like, when
   the actor is not trying to imitate humans?* Human-derived data is
   acceptable during bootstrap, not in the steady-state loop.

7. **General actor interface.** The system should accept any actor that
   maps `(state, move_space) → ranked_moves`: a frontier LLM, a small
   distilled model, a symbolic search, or a co-trained neural policy. The
   harness is the same.

---

## Milestone 1 — Define the Representation Stack

**Strategic role:** Fixes the coordinate system. Every later milestone is
parameterized by this choice. Cost of getting it wrong is highest because
it propagates.

**Decisions to make:**

- Which IR levels are ingested in v1 vs. deferred. Source and AST are
  given (cstat already does this). MIR and LLVM IR require either rustc
  internals (unstable) or a stable bridge (e.g. `cargo rustc -- --emit`,
  `llvm-tools-preview`, or compiler-as-library via stable JSON dumps).
  Symbol table and DWARF are accessible via `nm` / `objdump` /
  `gimli` on debug builds.
- The oracle interface: every level contributes named scalars to a single
  flat state vector. Levels are independent ingestion modules; missing
  levels degrade the vector gracefully (NaN or omitted, not zero).
- The move-tagging convention: every move declares a set of representation
  levels it reads from (preconditions) and writes to (effects), even if
  the actual edit is source-text. This is what later lets a planner
  reason about which moves are cheap to predict vs. which require
  re-running the oracle.

**Done when:** A spec document and an empty-but-typed oracle interface
exist. Concrete metric implementations can land later; the *shape* is
fixed.

**Foundational dependency:** Nothing precedes this.

---

## Milestone 2 — Inventory the Move Space, Per Level

**Strategic role:** Establishes what actually acts on the state vector.
Determines whether a symbolic DSL is feasible or whether moves are too
ad-hoc.

**Approach:** Discover, do not enumerate by intuition. Apply candidate
moves on real targets, observe what the vector does, keep what produces
predictable, well-typed deltas.

**Per-level expectations (anticipated, to be verified):**

- *AST-level:* extract function, inline function, merge functions, delete
  unreachable function, move function across modules, split module,
  replace nested match with early returns, parameterize a struct.
- *HIR/MIR-level:* dead basic-block elimination after lowering, collapse
  redundant match arms, simplify control flow that's only redundant
  post-desugar.
- *LLVM IR / codegen-level:* devirtualization opportunities, monomorph
  pressure (replace generic with dyn, or the reverse, based on
  source-to-symbol ratio).
- *Cross-level:* a single source edit (generic → dyn) can produce small
  AST delta and large symbol-table delta. These moves carry the most
  information per edit.

**Move record format:** name, level tags, preconditions (queryable from
oracle), predicted effect on state vector (which components, predicted
sign, predicted magnitude class), and confidence (deterministic vs.
empirical).

**Done when:** A typed catalog of ~10–20 moves exists, with at least one
empirical trajectory per move showing the vector delta it produces on a
real target.

**Depends on:** Milestone 1.

---

## Milestone 3 — Profile LLM Cleanup Behavior

**Strategic role:** Tells us where the LLM is reliable and where the
eventual non-LLM optimizer must cover. Without this, the symbolic side is
designed against an imagined LLM, not a real one.

**Method:** Controlled cleanup sessions on messy targets, varying scope:

- Single function (local complexity reduction).
- Single file (intra-module restructuring).
- Cross-module (move/split, dependency detangling).
- Multi-step coordinated (3+ moves where intermediate states look neutral
  or worse).

**Recorded per session:** moves proposed, moves applied cleanly, moves
that broke build/tests, vector deltas, where the LLM plateaued or
hallucinated. Multiple actors, multiple seeds, multiple targets.

**Output:** A capability profile — per move type and per scope, how
reliably the LLM produces a clean, correct, vector-improving edit. This
profile is what the optimizer in #7 is designed *against* (covers gaps,
delegates strengths).

**Depends on:** Milestone 2 (so we know what moves to evaluate). Can run
in parallel with #2 once #2 has an initial move set.

---

## Milestone 4 — Design the Test Methodology

**Strategic role:** Gates everything downstream that requires measurement.
Bad methodology produces unfalsifiable results.

**Requirements:**

- Reproducible. Same target, same starting code, same metric, same
  result.
- Non-human-judged. Success is defined by behavioral conformance, not
  taste.
- Behavioral spec is independent of structural metric. Tests evaluate
  *what the code does*, the vector evaluates *how it is built*. The two
  must not collapse into each other.
- Scope-graduated. Tiny targets (toy VM) for fast iteration; medium
  targets (Chip-8, NES subset, JSON parser, regex engine) for
  trajectory-shape evidence; larger targets (something approaching but
  below GBA emulator scope) noted as future once smaller scopes are
  reliable.

**Methodology design produced now, before any large-scope target is
attempted.** Specifically: scoring protocol, trajectory definition
(sequence of (state, move, new_state, behavior_test_result) tuples),
cross-run comparison method, and what counts as a "completed" cleanup
run.

**Strategic note on the emulator idea:** This is the eventual stress test.
"Build XYZ from a spec" is reliably accomplished by current LLMs at
sub-GBA scope. Pairing the cleanup tool against an LLM-built emulator
gives us a target where (a) behavioral correctness is mechanically
checkable via a conformance suite, (b) the initial code quality is
representative of agent-generated mess, and (c) the scope is large enough
that non-trivial restructuring matters. Note for future; the methodology
design happens now, the large-target runs happen post-#6.

**Depends on:** Milestone 1 (state vector defined), #2 (moves defined for
trajectory representation).

---

## Milestone 5 — Design Data Collection

**Strategic role:** Captures the substrate that everything after #6 is
built on. Insufficient logging is unrecoverable; over-logging is harmless.

**Constraint:** Minimize human-corpus reliance inside the loop (principle
#6 above). Bootstrap from existing LLM behavior is acceptable; ongoing
training data should come from the loop's own operation on the test
targets from #4.

**Logged per step:**

- Full state vector before and after.
- Representation-level snapshots referenced by the oracle (cheaply: which
  IR levels contributed, what their summary scalars were).
- Proposed move (or moves, if multiple were ranked).
- Applied move's diff.
- Level tags on the applied move.
- Behavioral test result.
- Actor identity (which LLM, which prompt strategy, which seed) and
  optionally chain-of-thought / explanation when the actor produces one.
- Wall-clock and oracle-recompute cost.

**Not logged:** Pre-stored human refactor exemplars used as targets. We
want trajectories shaped by the metric, not by mimicry of canonical
human-authored diffs. (If a target *happens* to be a human codebase, fine
— but the actor's training signal should be the oracle, not the human's
historical edits.)

**Depends on:** Milestones 1 (vector shape) and 2 (move shape). Runs in
parallel with #4.

---

## Milestone 6 — Bootstrap Loop, End-to-End

**Strategic role:** Produces the empirical substrate for every subsequent
design decision. *Not* intended to produce clean codebases at this stage —
intended to produce trajectories.

**Configuration:**

- Actor: LLM (frontier, well-instructed).
- Oracle: full representation stack from #1.
- Move space: catalog from #2.
- Targets: graduated set from #4.
- Logging: per #5.

**Runs:** Multiple targets × multiple seeds × multiple prompt strategies.
Both successful and failed trajectories are useful.

**Outputs:**

- A trajectory dataset.
- An empirical answer to "how often does a single move improve the vector
  vs. requires a coordinated sequence." This is the locality question
  that drives algorithm choice in #7.
- An empirical answer to "where does the LLM plateau, and which axes does
  it never touch." This shapes the optimizer's coverage requirements.
- Goodhart-resistance evidence: run at least one degenerate prompt
  ("minimize the vector, ignore behavior except to keep tests passing")
  and inventory the exploits found. This is methodology stress-testing,
  not a separate milestone.

**Done when:** Sufficient trajectory data exists to make the design
choices in #7 evidence-based rather than guesswork.

**Depends on:** All of 1–5.

---

## Milestone 7 — Optimizer Design Discussion

**Strategic role:** Commits to a mechanism. First non-reversible point in
the roadmap.

**Inputs from earlier milestones:**

- Move catalog with level tags, preconditions, predicted effects (#2).
- LLM capability profile, with strengths and gaps (#3).
- Trajectory dataset showing locality / non-locality (#6).
- Goodhart exploit inventory (#6).

**Decisions to make:**

- Algorithm class. Candidates include greedy descent, beam search,
  simulated annealing on the Pareto frontier, MCTS over move sequences,
  neuro-symbolic policy (symbolic move enumeration + learned move-prior
  net). Choice is informed by the locality evidence from #6, not by
  intuition.
- Where the LLM sits in the final system: primary actor with optimizer as
  guardrail, primary optimizer with LLM as move-proposer for moves
  outside the catalog, or full delegation to a co-trained policy with
  LLM available only as fallback.
- Training regime if a policy net is included. The trajectory dataset
  from #6 is the seed; whether on-policy data collection continues in
  steady state is a sub-decision.

**Done when:** An architecture is selected and a v1 build is specified.
Build itself is post-roadmap.

**Depends on:** All previous milestones.

---

## Dependency Graph

```
            #1 (representation stack)
              │
        ┌─────┴─────┐
        ▼           ▼
       #2          #3
   (moves)    (LLM profile)
        │           │
        └─────┬─────┘
              ▼
        ┌─────┴─────┐
        ▼           ▼
       #4          #5
  (test method)  (data collection)
        │           │
        └─────┬─────┘
              ▼
              #6
       (bootstrap loop)
              │
              ▼
              #7
       (optimizer design)
```

#1 strictly first. #2 and #3 in parallel. #4 and #5 in parallel after
#2/#3. #6 after all of 1–5. #7 after #6.

---

## Parked / Future

These are noted because they belong on the long horizon but should not
distract from the milestone sequence above.

- **Co-training a neural policy with the symbolic optimizer.** Possible
  only after #7's architecture is built and #6's data collection is in
  steady state. Treat as a v2 concern.
- **Generalization across languages.** cstat is Rust-only. The
  representation stack from #1 should not preclude other languages, but
  v1 is Rust.
- **Temporal / git-history metrics.** Deferred per
  `improvements-discussion.md`. May re-enter as a data source for the
  bootstrap phase only (per principle #6) if useful.
- **Agent-vs-human style classification.** Out of scope for the
  optimizer, in scope for cstat as a measurement tool.
- **Emulator-class targets.** Designed for in #4, executed against in a
  post-#6 follow-up. The methodology is built early; the large runs come
  later.
- **Human-codebase Pareto comparison.** Compare optimizer-cleaned
  codebases against the same codebases as cleaned by human maintainers,
  on the structural vector. Not for validation (we explicitly do not
  calibrate against human taste) but for characterization of where the
  two diverge.

---

## Status

Document drafted; no milestones started. Milestone #1 is the next
deliverable.
