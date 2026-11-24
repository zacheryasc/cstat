# Grind Corpus Testbed Plan

Grounding:

- `PROJECT_GROUNDING.md` frames `cstat` around codebase-shape exploration, a deterministic complexity/cleanup score, and an RLVF-style harness/environment.
- Current `grind` in `~/yoke` initializes `.loop/yoke.conf`, `.loop/protocol.md`, `.loop/grind-gate.py`, `.loop/judge.md`, and `.loop/cstat-baseline.json`.
- The current grind gate runs the behavior oracle, optional benchmark command, and `cstat --path . --json scorecard`; it passes when current `code_complexity_cost` is lower than the baseline.
- The evaluation corpus lives under `~/data/code-analysis-evaluation/foreign-repositories` and contains third-party pinned Rust repositories. Treat those checkouts as immutable inputs; run grind in branches/worktrees.

Key design correction: do **not** use `grind-passes 5` as the first automation shape. Current `grind` compares every pass against the original `.loop/cstat-baseline.json`; after the first improvement, later passes can pass without further improvement. Use five external one-pass runs instead, refreshing the baseline before each pass.

## 1. Keep orchestration outside `cstat`/`yoke` first

`PROJECT_GROUNDING.md` says the project is still polishing measurement primitives before committing to full RLVF targets. So make this an external experiment driver, not a permanent product surface yet.

Suggested driver name:

```text
grind-corpus-runner
```

Responsibilities:

1. select repo from `~/data/code-analysis-evaluation/foreign-repositories`;
2. create an isolated git branch/worktree;
3. ask a read-only settings agent for per-repo `yoke.conf` settings;
4. initialize `yoke init grind`;
5. apply settings;
6. run 5 one-pass grind attempts with rolling baselines;
7. run deterministic evaluator;
8. write machine-readable result artifacts.

No changes to pinned corpus checkouts in place.

## 2. Settings discovery agent

Make this agent **read-only**. It should not edit `.loop/yoke.conf`; the controller applies its output.

### Input

```json
{
  "repo": "async-trait",
  "path": "/home/aaron/data/code-analysis-evaluation/foreign-repositories/async-trait",
  "mode": "grind-settings-discovery"
}
```

### Agent tasks

For each repo:

1. Inspect:
   - `Cargo.toml`;
   - workspace members;
   - top-level and member-level `tests/`, `benches/`, `benchmarks/`, `examples/`, `fixtures/`, `snapshots/`;
   - `.github/`, `Makefile`, `justfile`, README/CONTRIBUTING for test commands;
   - library/public API risk.
2. Run preflight candidates, strongest first:
   - `cargo test --workspace --all-targets --all-features`;
   - if too broad/failing at baseline, narrower workspace/package test;
   - fallback `cargo check ...` only if tests are not runnable, and mark low confidence.
3. Decide:
   - `GRIND_ORACLE`;
   - optional `GRIND_BENCH`;
   - protected prefixes for `no-modify`;
   - whether public API compatibility checks are needed;
   - timeout/confidence/risk notes.

### Output schema

```json
{
  "repo": "async-trait",
  "baseline_commit": "...",
  "oracle_command": "cargo test --all-targets --all-features",
  "bench_command": "",
  "cstat_command": "cstat --path . --json scorecard",
  "protected_prefixes": [
    "tests/",
    "examples/",
    "fixtures/",
    "snapshots/"
  ],
  "api_guard": {
    "required": true,
    "reason": "library crate with public API",
    "command": "cargo semver-checks check-release --baseline-rev <base>"
  },
  "preflight": {
    "oracle_passed": true,
    "seconds": 12.4
  },
  "confidence": "medium",
  "risks": [
    "inline #[cfg(test)] modules cannot be protected by prefix-only yoke rules"
  ],
  "evidence": [
    "Cargo.toml has [package]",
    "tests/ exists",
    "README documents cargo test"
  ]
}
```

Important: the controller rejects settings output if it weakens the harness, e.g. no oracle, no protected behavior dirs when they exist, or fallback to `cargo check` without recording low confidence.

## 3. Initial guard-prefix candidates from corpus inspection

Observed candidate `no-modify` prefixes:

| repo | candidate prefixes | notes |
|---|---|---|
| async-trait | `tests/` | test-heavy scorecard |
| bat | `tests/`, `examples/` | cstat had 2 parse-error files; require parse errors not increase |
| boa | `tests/`, `benches/`, `examples/` | cstat timed out at 90s; needs longer timeout or skip initially |
| dustbox-rs | `dustbox/benches/` | tests appear inline/member-level |
| fd | `tests/` | good early candidate |
| fdb | `fdb/tests/` | good early candidate |
| image | `tests/`, `benches/`, `examples/` | large but tractable |
| miners | none found | small softball candidate |
| naive-query-engine | none found | inline tests detected by cstat |
| poem | `examples/`, `poem-mcpserver/tests/`, `poem-openapi/tests/` | larger workspace |
| realworld-axum-sqlx | none found | app oracle likely needs env/db care |
| runk | `examples/` | small candidate |
| rust-libp2p | `examples/`, `core/tests/`, `identity/tests/`, `identity/benches/`, `swarm/tests/`, `swarm/benches/` | large workspace |
| serde_json | `tests/` | library/API guard required |
| starship | none found | inline tests; large CLI |

Start with small/medium repos:

```text
miners
fdb
runk
async-trait
fd
naive-query-engine
```

Defer:

```text
boa          # scorecard timeout
rust-libp2p  # very large workspace
realworld-axum-sqlx # likely external service/env coupling
```

## 4. Branch/worktree strategy

Corpus metadata says these are third-party pinned inputs; avoid editing in place.

For each run:

```text
source = ~/data/code-analysis-evaluation/foreign-repositories/<repo>
runroot = ~/data/code-analysis-evaluation/grind-runs/<timestamp>/<repo>
worktree = <runroot>/worktree
branch = yoke-grind/<timestamp>/<repo>
```

Controller does:

```sh
git -C "$source" worktree add -b "$branch" "$worktree" HEAD
```

Then all mutation happens in `$worktree`.

The grind worker remains forbidden from git operations per grind protocol. The controller owns branching, diffs, artifact capture, and cleanup.

Do not require commits. If existing git identity is configured, controller may commit pass snapshots; otherwise store patches:

```text
pass-1.diff
pass-2.diff
...
final.diff
```

This avoids modifying git config.

## 5. Yoke setup per repo

Inside the worktree:

```sh
yoke init grind
```

Then controller overwrites `.loop/yoke.conf` from the settings output.

Template shape:

```text
max-tail 200
grind-passes 1

allow .

no-modify tests/
no-modify benches/
no-modify benchmarks/
no-modify examples/
no-modify fixtures/
no-modify snapshots/

guard python3 .loop/grind-gate.py

metrics-dir /home/aaron/data/code-analysis-evaluation/grind-runs/<run-id>/<repo>/metrics
```

Do not rely on commented defaults. Active `no-modify` lines must be explicit.

The controller should set env vars at run time:

```sh
GRIND_ORACLE="<settings oracle>" \
GRIND_BENCH="<settings bench or empty>" \
GRIND_CSTAT="<settings cstat command>" \
yoke run
```

## 6. Five-pass rolling-baseline loop

Use `grind-passes 1` and run `yoke run` five times.

For pass `N`:

1. Capture before score:

   ```sh
   cstat --path . --json scorecard > artifacts/pass-N/before-scorecard.json
   cp artifacts/pass-N/before-scorecard.json .loop/cstat-baseline.json
   ```

2. Run one grind pass:

   ```sh
   GRIND_ORACLE="..." GRIND_BENCH="..." yoke run
   ```

3. Capture after score:

   ```sh
   cstat --path . --json scorecard > artifacts/pass-N/after-scorecard.json
   ```

4. Capture:
   - `.loop/guard-results.md`;
   - `.loop/notes.md`;
   - `.loop/verdict.md` if used;
   - full diff from previous pass;
   - cumulative diff from branch base;
   - command timings/statuses.

5. Deterministic evaluator decides whether pass is accepted.

If a pass fails hard, stop repo run and record terminal reason. Do not continue from a broken state unless the plan explicitly wants recovery behavior.

## 7. Objective evaluator

Primary judge should be a deterministic script, not an LLM.

### Hard gates

A pass is rejected if any are true:

1. `GRIND_ORACLE` failed.
2. `GRIND_BENCH` failed when configured.
3. `cstat` failed or emitted invalid JSON.
4. `score_version` changed unexpectedly.
5. `metadata.parse_error_files` increased.
6. `code_complexity_cost` did not decrease from that pass’s rolling baseline.
7. Any protected prefix changed.
8. `.loop/protocol.md`, `.loop/yoke.conf`, `.loop/grind-gate.py`, `.loop/cstat-baseline.json` changed by the worker.
9. Public API compatibility check failed for library crates.
10. Doctests/examples check failed when configured.
11. Tests, fixtures, snapshots, examples, or docs were changed to hide behavior.

### Scoring for accepted passes

Use within-repo scalar improvement:

```text
absolute_delta = before.code_complexity_cost - after.code_complexity_cost
relative_delta = absolute_delta / before.code_complexity_cost
```

Do not compare raw `code_complexity_cost` across repos. For cross-repo summaries use:

```text
median relative_delta
success rate
accepted passes / attempted passes
final relative_delta from original baseline
code_complexity_cost_per_kloc as descriptive context only
```

### Anti-gaming checks

For libraries, add at least one API gate before trusting results:

- `cargo semver-checks`, or
- `cargo public-api` before/after diff, or
- generated downstream compile probe.

This is required by the observed grind failure mode: lower `code_complexity_cost` can be achieved by deleting public API while tests still pass.

## 8. Result artifact

Each repo run should produce one canonical JSON file:

```json
{
  "run_id": "2026-07-25T...Z",
  "repo": "async-trait",
  "source_commit": "...",
  "branch": "yoke-grind/...",
  "settings": {
    "oracle_command": "...",
    "bench_command": "",
    "protected_prefixes": ["tests/"],
    "api_guard": true
  },
  "passes": [
    {
      "pass": 1,
      "status": "accepted",
      "before_cost": 586.1,
      "after_cost": 540.3,
      "absolute_delta": 45.8,
      "relative_delta": 0.0781,
      "oracle_passed": true,
      "bench_passed": null,
      "protected_changes": [],
      "parse_errors_before": 0,
      "parse_errors_after": 0,
      "api_guard_passed": true,
      "reason_codes": []
    }
  ],
  "final": {
    "status": "accepted",
    "accepted_passes": 5,
    "attempted_passes": 5,
    "original_cost": 586.1,
    "final_cost": 490.0,
    "relative_delta": 0.164
  }
}
```

And a corpus summary:

```json
{
  "repos_attempted": 6,
  "repos_accepted": 4,
  "repos_rejected": 2,
  "median_final_relative_delta": 0.08,
  "median_accepted_passes": 4,
  "failures_by_reason": {
    "oracle_failed": 1,
    "api_break": 1
  }
}
```

## 9. Optional LLM judge

Use yoke’s `judge-every 1` only as a secondary audit, not the primary score.

Good use:

```text
qualitative_maintainability_review = advisory
objective_result = deterministic evaluator
```

If the LLM judge fails but deterministic gates pass, label the run:

```text
accepted_with_review_warning
```

Do not let a subjective judge define the benchmark score.

## 10. First implementation slice

Build the smallest useful automation first:

1. Driver handles one repo.
2. Settings agent emits JSON only.
3. Controller applies `yoke.conf`.
4. Run 5 one-pass rolling-baseline grinds.
5. Deterministic evaluator produces `result.json`.
6. Try on `miners`, `fdb`, `runk`.
7. Expand to `async-trait`, `fd`, `naive-query-engine`.
8. Only then add API guards and broader corpus runs.

This keeps the experiment aligned with the grounding doc: prove the RLVF loop externally, gather evidence, then decide what belongs in `cstat` or `yoke` permanently.
