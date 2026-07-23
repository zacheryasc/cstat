# cstat Grounding Plan

## 1. Project Intent

`cstat` is a Rust codebase-shape analysis tool. It exists to make messy Rust codebases easier to inspect, reason about, and improve in bounded steps.

The project should prove three linked ideas:

1. **Codebase shape exploration** — existing probes expose different structural views of a Rust codebase.
2. **Verified complexity function** — those probes can feed a deterministic, explainable complexity/cleanup function.
3. **RLVF harness / environment** — the tool can support baseline, attempted cleanup, verification, re-measurement, and structured feedback for human or agent-driven codebase fixing.

This project is not trying to become a general-purpose static analyzer, automatic refactoring engine, full RL training system, architecture platform, or web product.

## 2. End-State Goals

### 2.1 Codebase Shape Exploration

`cstat` should let a user explore the shape of a Rust codebase through multiple static lenses. Existing probes are valuable because each one describes a different part of codebase shape.

Relevant shape views include:

- line/file size and distribution;
- symbol counts and concentration;
- module dependencies and coupling;
- per-function and per-file complexity;
- static dead-code candidates;
- static test/benchmark reachability;
- call-flow or call-trace views where useful;
- coverage or cluster views where they directly support exploration or verification.

The goal is not to hide these probes. The goal is to make clear why they exist, when to use them, and how they contribute to the larger project purpose.

### 2.2 Verified Complexity Function

`cstat` should define one deterministic complexity or cleanup function derived from existing metrics.

This function should be:

- deterministic for the same input codebase;
- explainable from its component metrics;
- stable enough to consume from JSON output;
- tested against fixtures and invariants;
- honest about what it measures.

“Verified” does not mean the function proves true code quality. It means the function has a stated contract, stable inputs and outputs, and tests showing that obvious structural changes move the score in expected directions.

Example invariants:

- adding branches should not reduce the complexity component;
- adding large functions should not improve the size/complexity result;
- adding unnecessary dependencies should not improve coupling;
- reducing test reachability should not improve the verification signal;
- preserving behavior while lowering measured complexity should improve the cleanup signal.

### 2.3 RLVF Harness / Environment

`cstat` should support an RLVF-style loop for codebase-fixing attempts.

The minimal loop is:

1. establish a baseline shape/complexity measurement;
2. allow a human or agent to make a bounded cleanup attempt;
3. run the project’s verification command, such as tests or build;
4. re-measure shape and complexity;
5. emit structured feedback describing whether the attempt improved the measured objective while preserving required behavior.

The harness does not need to train a model. It does not need to call an LLM. It does not need to apply patches automatically. For this project, the harness is the environment and feedback mechanism around a codebase-changing attempt.

## 3. Boundaries / Non-Goals

The MVP stays bounded by these rules:

- Rust-only for now.
- Static-first analysis.
- Existing probes should be organized before new probes are added.
- New analysis is only in scope if it directly supports shape exploration, the complexity function, or the RLVF harness.
- No automatic refactoring.
- No RL training loop.
- No LLM integration.
- No promise of perfect macro expansion, dynamic dispatch resolution, or runtime truth.
- No web UI.
- No plugin system.
- No multi-language support.
- No broad benchmark suite before the core loop works.
- No feature is justified merely because it is interesting.

A change is in scope only if it improves one of these:

- codebase shape exploration;
- the verified complexity function;
- the RLVF harness/environment;
- the demo proving those concepts;
- the documentation needed to make the project understandable.

## 4. Current Assets

The project already has substantial useful material:

- multiple CLI probes for structural codebase analysis;
- focused root commands for each maintained analysis path;
- JSON output modes;
- tests around several CLI behaviors;
- static analysis modules for size, symbols, dependencies, complexity, dead-code candidates, reachability, call flow, coverage, clustering, and related views;
- `cstat-agent-use.md`, which already points toward agent-oriented tool use;
- enough real code for dogfooding the tool on itself.

These assets should be treated as the foundation, not as clutter to discard by default.

## 5. Current Gaps

The missing pieces are mostly framing, contracts, and end-to-end proof.

### 5.1 Purpose and CLI Organization

The CLI exposes many capabilities, but the project purpose is not yet clearly organized around shape exploration, complexity scoring, and RLVF feedback.

The probes work, but they need to be presented as intentional views of codebase shape rather than as unrelated commands.

### 5.2 Verified Complexity Function

The project does not yet define a single named complexity/cleanup function with:

- explicit metric inputs;
- a deterministic formula;
- a stable JSON output shape;
- stated invariants;
- tests proving those invariants.

### 5.3 RLVF Harness Flow

The project does not yet demonstrate the full loop:

- baseline measurement;
- bounded cleanup attempt;
- project verification;
- after measurement;
- structured feedback/reward evidence.

Without this loop, the RLVF claim is only conceptual.

### 5.4 Demo

The project needs a concrete demo showing that the tool works end to end.

The demo should show:

- human exploration of a messy codebase;
- machine-readable output suitable for an agent;
- a before/after verification path;
- how the complexity function changes after a bounded cleanup.

### 5.5 Public Explanation

The README is too thin to communicate what the project is, why it exists, or how the existing pieces fit together.

The project needs enough surface-level polish that another developer can understand:

- what problem `cstat` solves;
- what commands or profiles to start with;
- what the metrics mean;
- what the limitations are;
- how the demo proves the core claims.

## 6. Gap-Fill Plan

### 6.1 Organize Existing Probes

Document the existing probes as codebase-shape views.

For each probe, clarify:

- what shape dimension it exposes;
- whether it is mainly useful for humans, agents, RLVF feedback, or all three;
- what command produces it;
- whether the output is stable enough for machine use.

This should make the existing breadth feel intentional without requiring every probe to become part of the core proof.

### 6.2 Define Tool-Use Profiles

Define limited profiles that combine existing probes for specific workflows.

Candidate profiles:

- **human exploration profile** — emphasizes readable summaries and cleanup starting points;
- **agent steering profile** — emphasizes compact JSON, hotspots, constraints, and suggested drilldowns;
- **RLVF profile** — emits stable fields needed for before/after feedback and reward calculation.

Profiles should be thin orchestration over existing analysis where possible.

### 6.3 Add the Complexity Function

Define a named complexity or cleanup cost function.

The implementation should reuse existing metrics first. The first version should prioritize determinism, explainability, and tests over sophistication.

The output should include:

- score or cost value;
- score version;
- component breakdown;
- top contributors/hotspots;
- enough metadata to compare before/after results.

### 6.4 Verify the Complexity Function

Add tests that prove the function’s contract.

The tests should focus on observable properties, not incidental implementation details.

Useful test cases include:

- a small/simple fixture has lower cost than a deliberately messy fixture;
- adding control-flow branches increases or preserves complexity cost;
- adding dead private functions worsens the relevant component;
- reducing test reachability does not improve the RLVF signal;
- JSON output contains the expected stable fields.

### 6.5 Build the RLVF Harness Flow

Create the minimal harness path for codebase-fixing attempts.

The harness should produce an artifact that records:

- task or attempt metadata;
- baseline profile output;
- verification command and result;
- after profile output;
- complexity delta;
- feedback/reward signal;
- reasons for acceptance or rejection.

The harness should remain separate from model training or patch generation.

### 6.6 Add an End-to-End Demo

The demo should prove the project’s core claims without becoming a second product.

A good demo path is dogfooding `cstat` on itself:

1. run shape exploration on the current repo;
2. identify an obvious cleanup target from the output;
3. make or describe a bounded cleanup attempt;
4. run project verification;
5. re-run the relevant profile;
6. show the feedback artifact.

If dogfooding is too noisy, add a small fixture crate that intentionally contains a few simple forms of messiness.

### 6.7 Polish Public Explanation

Update the public explanation after the core loop exists.

The README should explain:

- the three project goals;
- the basic commands or profiles;
- one short demo;
- what the metrics do and do not mean;
- how humans and agents use the tool differently;
- the non-goals that keep the project bounded.

## 7. Completion Criteria

The MVP is complete when all of the following are true:

- The project can be explained in one paragraph.
- Existing probes are framed as codebase-shape views.
- A deterministic complexity function exists.
- The complexity function has stated inputs, output, and invariants.
- Tests verify the complexity function’s basic contract.
- An RLVF-style harness flow exists.
- The harness produces structured feedback for a before/after cleanup attempt.
- A demo shows shape exploration, complexity scoring, verification, and feedback.
- The README explains the project clearly enough for a new developer to try it.
- Non-goals are documented and used to reject unrelated scope.

## 8. Scope Guardrail

The project should not expand just because another metric, command, or dashboard would be interesting.

The guardrail is:

> If a change does not support codebase shape exploration, the verified complexity function, the RLVF harness, the demo, or the public explanation of those pieces, it is outside the MVP.

## 9. Current CLI Polish Pass

We are currently going through the existing CLI commands and probes one by one. The goal is to polish, clarify, and prune the extant surface before designing RLVF targets or higher-level harness behavior.

This pass is not about adding new analyzers. It is about deciding, for each existing command:

- what codebase-shape concept it measures;
- whether the measure is useful for humans, agents, and later RLVF workflows;
- whether the output is granular enough;
- whether the behavior is tested well enough;
- whether the CLI/help/docs explain it clearly;
- whether any parts should be renamed, folded into another concept, demoted, or dropped.

Only after this CLI/probe pass is complete should we design the RLVF target or targets, because the RLVF harness needs stable, well-understood measurement primitives.

## 10. Current Branch / Worktree Status

- Main line now includes the accepted `loc` polish work.
  - `loc` is a first-class line/size-shape probe.
  - The `loc` branch/worktree was folded in and removed.
- Main line now includes the accepted `symbols` polish work.
  - `symbols` is a first-class symbol-shape probe.
  - It clarifies impl/trait-impl semantics, selected-file behavior, JSON output, and symbol granularity.
  - The `symbols` branch/worktree was folded in and removed.
- `-complexity` branch:
  - The old broad `complexity` command has been sharded.
  - The sharding direction is accepted.
  - Each new individual item still needs review before it is ready to fold in.
  - Current stopping point: complexity concept split exists, but the resulting probes are not yet accepted as polished.

## 11. Stopping Point

- We resumed the CLI polish/pruning pass long enough to fold in the accepted `loc` and `symbols` work.
- `loc` and `symbols` are now the first accepted polished probes on the main line.
- We agreed that the old `complexity` command was too broad and vague.
- We narrowed the intended complexity split to specific probes from the existing complexity behavior:
  - branching/control-flow path complexity;
  - signature/type-boundary complexity;
  - span/function-body size.
- We agreed not to treat all codebase complexity as one vague command.
- Current next review point is the sharded `-complexity` worktree/branch.

## 12. Next Steps

1. Review the sharded `-complexity` items individually.
2. Accept, revise, or reject each complexity shard based on whether it is granular, objective, tested, and useful for human/agent/RLVF workflows.
3. Fold in only the accepted complexity shards.
4. Continue the same polish/pruning review for the remaining existing CLI commands.
5. After the measurement primitives are stable, design the RLVF target or targets around those accepted probes.

## 13. Extreme Bare Minimum

Before the broader MVP work, the project needs a narrow, polished, resume-linkable slice that proves `cstat` is already usable.

This slice is not the full verified complexity function or RLVF harness. It is the smallest public path that shows a clean CLI and a credible human/agent demo.

### 13.1 Polished CLI MVP

The CLI MVP should expose a small accepted command surface rather than every existing probe.

For this slice:

- `loc` is folded in as an accepted line/size-shape probe;
- `symbols` is folded in as an accepted symbol-shape probe;
- present both as first-class codebase-shape probes;
- make command names, help text, examples, and output modes clear enough for a new user to run without project context;
- keep JSON output stable enough for agent consumption;
- hide, demote, or leave out unfinished rough commands from the public happy path.

The goal is a CLI that feels intentional and usable, not a complete analysis platform.

### 13.2 Human Demo

Add a short demo showing how a person uses the polished CLI on a Rust repo.

The demo should show:

- the exact commands to run;
- how to read file/size shape output;
- how to read symbol concentration output;
- how those outputs point to likely cleanup targets;
- one brief investigation or before/after walkthrough.

### 13.3 AI-Agent Demo

Add a short demo showing how an AI agent should consume the polished CLI.

The demo should show:

- the exact commands an agent should run;
- the JSON output path or mode it should consume;
- how to identify hotspots from structured output;
- how to choose a bounded next probe or cleanup target from that evidence.

This demo should stop at agent steering. It does not need the full RLVF harness, model training, automatic patching, or reward loop.

### 13.4 Completion Criteria

The extreme bare minimum is complete when:

- the polished CLI path is narrow, documented, and runnable;
- `loc` and `symbols` are folded into the main line as accepted probes;
- rough unfinished commands are not part of the primary public path;
- a human demo explains how to use the output;
- an AI-agent demo explains how to consume the JSON and pick a bounded next action;
- the README can support a resume link without implying the broader MVP is finished.
