# Harness Design Needs

## Distinguishing Verification Cases and Freezing Behavior

The harness should not treat all tests as equal. A refactor may change implementation structure, but it must preserve observable behavior. The harness therefore needs to separate behavioral oracles from implementation-coupled checks.

### Behavioral Oracle

A behavioral oracle is any check that verifies public or externally observable behavior.

Examples:

- CLI tests that assert exit code, stdout, stderr, generated files, or error messages.
- Integration tests under `tests/` that exercise the crate as an external caller.
- Doctests that document public API behavior.
- Golden/snapshot tests for stable user-visible output.
- Public API scenario tests.
- Serialization/deserialization compatibility tests.
- Parser/interpreter input-output tests.
- Example programs that are used as supported behavior.

These checks should be frozen for the attempt. The model should not be allowed to edit, delete, weaken, or skip them. Ideally they live outside the editable repo copy during evaluation.

### Internal / Structural Tests

Internal tests verify how the current implementation works, not necessarily what the public surface promises.

Examples:

- `#[cfg(test)]` tests inside private modules.
- Tests for helper functions that are not public API.
- Tests asserting intermediate structs, private enum variants, module boundaries, or exact control flow.
- Tests that mainly defend a current decomposition.

These should be treated as advisory, not automatically frozen. A successful refactor may legitimately delete, rewrite, or obsolete them.

However, if an internal test encodes behavior that is not otherwise covered, the right response is not to freeze the internal test. The right response is to promote the behavioral claim into the behavioral oracle, then allow the internal test to change.

### Ambiguous Tests

Some tests are mixed: structurally coupled, but guarding real behavior.

The harness should classify these as ambiguous until resolved.

Resolution rule:

```text
If the test protects externally observable behavior,
extract or reinforce that behavior as a frozen oracle check.
Then treat the original implementation-coupled test as advisory.
```

This prevents two failures:

1. freezing brittle internal structure and blocking legitimate refactors;
2. unfreezing the only check that protected real behavior.

### Benchmarks

Benchmarks are not behavioral tests by default.

They answer:

- did performance regress?
- did allocation behavior change?
- did the refactor make a hot path worse?

Benchmarks should be classified separately as performance oracles.

For early harness work:

- compile-only benchmark checks may be required;
- runtime benchmark comparison may be advisory unless the task is performance-sensitive;
- serious regression should be reported even if behavior passes;
- benchmark code itself should usually be frozen if it represents a stable workload.

Acceptance should not depend on benchmark improvement unless the task says performance matters.

### Build Checks

Build checks are always required.

Examples:

- `cargo check`
- `cargo build`
- feature-matrix builds if relevant
- doctest compilation

These do not prove behavior, but failure means the attempt is invalid.

### Fixtures, Snapshots, and Golden Files

Fixtures and golden files must be classified by role.

Frozen if they define expected public behavior:

- CLI output snapshots;
- parser fixtures;
- protocol examples;
- compatibility data;
- public input/output examples.

Mutable or ignored if they are only internal scaffolding.

The dangerous case is a model “fixing” a failing test by updating the expected snapshot. The harness must reject edits to frozen snapshots unless the task explicitly allows behavior change.

### Proper Freezing Rule

The harness should freeze behavior contracts, not arbitrary files.

Good freezing:

```text
The same externally visible inputs produce the same accepted outputs.
```

Bad freezing:

```text
No file under tests/ may change.
```

Why bad: it blocks legitimate refactors and confuses implementation tests with behavior tests.

Better policy:

```text
Frozen:
  behavior oracle checks and their fixtures/snapshots

Mutable:
  implementation code
  internal structural tests
  obsolete helper tests

Reported separately:
  advisory tests
  benchmarks
  lints
```

## Loop Behavior

The harness runs one repository attempt at a time. Its job is to orchestrate the cleanup loop, not to decide code quality directly. Behavior preservation is judged by the frozen behavior oracle. Complexity improvement is judged by `cstat`. Performance preservation is judged by benchmarks when available.

A run has two phases: cleanup and repair.

### Cleanup Phase

The model is given:

- the repository;
- the task: reduce the `cstat` complexity score;
- instructions for using `cstat`;
- the final acceptance criteria;
- the frozen behavior and performance constraints;
- the cleanup turn limit.

For each cleanup turn, the model may inspect, edit, run local checks, and decide its next action. The harness does not need to force per-turn verification because models usually run local checks themselves. The cleanup phase ends when the model stops or the cleanup turn limit is reached.

### Final Verification

After cleanup, the harness runs the full final gate:

- frozen behavior oracle;
- full required verification tests;
- benchmarks if available;
- final `cstat` score.

The attempt is accepted only if:

```text
behavior verification passes
AND benchmark gate passes if benchmarks are available
AND final complexity score is lower than baseline
```

A benchmark failure means either the benchmark command fails or measured performance regresses beyond the configured tolerance.

### Repair Phase

If final verification fails, the harness gives the model a repair prompt containing the verification failure output. The model may then make focused fixes.

Repair is bounded by a repair turn limit. The spec requires such a limit to exist, but does not define a default value.

The repair phase exits when:

```text
final verification passes
OR repair turn limit is reached
```

If the repair turn limit is reached without passing verification, the attempt is rejected.

### Harness Boundary

The harness should not try to prevent every bad edit directly. It should rely on:

- frozen behavior oracle for behavior preservation;
- benchmark gate for performance preservation;
- `cstat` for complexity objective.

The harness itself only controls the loop, preserves the run boundary, supplies feedback, and records the final outcome.
