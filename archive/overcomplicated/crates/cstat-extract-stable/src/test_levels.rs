//! Spec §2.9 levels 48 (test inventory) and 49 (doc-test inventory).
//!
//! Drives `cargo test -- --list` into a private target directory. The
//! `--list` flag is honoured by libtest's standard runner: each test
//! binary prints its tests and a `"N tests, M benchmarks"` summary,
//! then exits without running anything.
//!
//! The cached invocation of `cargo test --no-run` (one per kind, one
//! per project) does the real work; the `--list` step is then cheap.
//! Both invocations share a target directory so we don't pay the
//! build cost twice.

use std::path::Path;
use std::process::Command;

use cstat_orchestrate::{Context, CostClass, Level, LevelError, LevelOutput};

// ───── Level 48: regular test inventory ────────────────────────────────

pub(crate) struct TestsLevel;

impl Level for TestsLevel {
    fn id(&self) -> &'static str { "tests" }
    fn cost_class(&self) -> CostClass { CostClass::B }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let target_dir = ctx.project_root.join("target").join("cstat-tests");
        let stdout = run_test_list(&ctx.project_root, &target_dir, TestKind::Unit)?;
        let summary = parse_listing(&stdout);

        let vector = vec![
            ("tests.binary_count".into(), summary.binary_count as f64),
            ("tests.test_count".into(), summary.test_count as f64),
            ("tests.benchmark_count".into(), summary.benchmark_count as f64),
            ("tests.test_function_count".into(), summary.test_names as f64),
            (
                "tests.ignored_count".into(),
                summary.ignored_names as f64,
            ),
        ];

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Level 49: doc-test inventory ────────────────────────────────────

pub(crate) struct DoctestsLevel;

impl Level for DoctestsLevel {
    fn id(&self) -> &'static str { "doctests" }
    fn cost_class(&self) -> CostClass { CostClass::B }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let target_dir = ctx.project_root.join("target").join("cstat-tests");
        let stdout = run_test_list(&ctx.project_root, &target_dir, TestKind::Doc)?;
        let summary = parse_listing(&stdout);

        let vector = vec![
            ("doctests.binary_count".into(), summary.binary_count as f64),
            ("doctests.test_count".into(), summary.test_count as f64),
            (
                "doctests.test_function_count".into(),
                summary.test_names as f64,
            ),
        ];

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

#[derive(Copy, Clone)]
enum TestKind {
    Unit,
    Doc,
}

fn run_test_list(project_root: &Path, target_dir: &Path, kind: TestKind) -> Result<String, LevelError> {
    let mut cmd = Command::new("cargo");
    cmd.arg("test");
    match kind {
        // `--all-targets` is lib + bins + tests + benches + examples,
        // explicitly EXCLUDING doctests — which we count separately
        // under level 49. Default `cargo test` would otherwise lump
        // both into one invocation and double-count.
        TestKind::Unit => {
            cmd.arg("--all-targets");
        }
        TestKind::Doc => {
            cmd.arg("--doc");
        }
    }
    cmd.arg("--target-dir")
        .arg(target_dir)
        .arg("--")
        .arg("--list")
        .current_dir(project_root);

    let output = cmd
        .output()
        .map_err(|e| LevelError::Skipped(format!("cargo not available: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let lc = stderr.to_ascii_lowercase();
        // libtest harnesses that aren't the standard one (e.g. nextest
        // when wired as the default test runner) don't understand
        // `--list`. Treat as skipped, not failed — the project isn't
        // broken, we just can't enumerate.
        if lc.contains("unrecognized") || lc.contains("found argument") {
            return Err(LevelError::Skipped(format!(
                "custom test harness does not support --list: {}",
                first_line(&stderr)
            )));
        }
        if lc.contains("no such command") || lc.contains("not installed") {
            return Err(LevelError::Skipped(format!(
                "cargo test unavailable: {}",
                first_line(&stderr)
            )));
        }
        return Err(LevelError::Failed(format!(
            "cargo test --list exited {:?}: {}",
            output.status.code(),
            stderr.lines().rev().take(6).collect::<Vec<_>>().join(" | ")
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("").trim()
}

#[derive(Debug, Default, PartialEq)]
struct TestListing {
    binary_count: u64,
    test_count: u64,
    benchmark_count: u64,
    test_names: u64,
    ignored_names: u64,
}

/// Parse libtest's `--list` output. The shape is one entry per test:
///
/// ```text
///   mod::name: test
///   mod::other: test (ignored)
///   mod::bench: benchmark
///
/// 2 tests, 1 benchmark
/// ```
///
/// Cargo runs each test binary sequentially, so the stream contains one
/// summary line per binary. We sum across all summaries to get the
/// project-wide counts; the per-line walk is the source of truth for
/// `test_names` / `ignored_names` since libtest is sometimes asked for
/// `--list` from a Cargo wrapper that suppresses the summary.
fn parse_listing(stdout: &str) -> TestListing {
    let mut out = TestListing::default();
    for line in stdout.lines() {
        let trimmed = line.trim();
        // Summary line: e.g. "3 tests, 0 benchmarks".
        if let Some((tests, benches)) = parse_summary(trimmed) {
            out.binary_count += 1;
            out.test_count += tests;
            out.benchmark_count += benches;
            continue;
        }
        // Per-entry lines look like "name: test" / "name: benchmark"
        // (sometimes with a trailing " (ignored)").
        if let Some(rest) = trimmed.strip_suffix(": test") {
            if !rest.is_empty() {
                out.test_names += 1;
            }
        } else if trimmed.ends_with(": test (ignored)") {
            out.test_names += 1;
            out.ignored_names += 1;
        }
    }
    out
}

fn parse_summary(line: &str) -> Option<(u64, u64)> {
    // The line format is `<N> tests, <M> benchmarks` (or with singular
    // `test`/`benchmark`). Anything else returns None.
    let mut parts = line.split(',').map(str::trim);
    let tests_part = parts.next()?;
    let bench_part = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let tests = leading_count(tests_part, &["test", "tests"])?;
    let benches = leading_count(bench_part, &["benchmark", "benchmarks"])?;
    Some((tests, benches))
}

fn leading_count(s: &str, suffixes: &[&str]) -> Option<u64> {
    let mut parts = s.split_whitespace();
    let n: u64 = parts.next()?.parse().ok()?;
    let tail = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if !suffixes.contains(&tail) {
        return None;
    }
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_summary_handles_singular_and_plural() {
        assert_eq!(parse_summary("0 tests, 0 benchmarks"), Some((0, 0)));
        assert_eq!(parse_summary("1 test, 0 benchmarks"), Some((1, 0)));
        assert_eq!(parse_summary("3 tests, 1 benchmark"), Some((3, 1)));
        assert_eq!(parse_summary("not a summary"), None);
        assert_eq!(parse_summary("3 things, 1 benchmark"), None);
    }

    #[test]
    fn parse_listing_aggregates_across_binaries() {
        let stream = "\
foo::a: test
foo::b: test (ignored)
foo::c: benchmark

2 tests, 1 benchmark

bar::x: test

1 test, 0 benchmarks
";
        let r = parse_listing(stream);
        assert_eq!(
            r,
            TestListing {
                binary_count: 2,
                test_count: 3,
                benchmark_count: 1,
                test_names: 3,
                ignored_names: 1,
            }
        );
    }

    #[test]
    fn parse_listing_handles_empty_input() {
        let r = parse_listing("");
        assert_eq!(r, TestListing::default());
    }

    #[test]
    fn parse_listing_handles_zero_tests_block() {
        // Doc-test mode with no doctests still emits a summary.
        let r = parse_listing("0 tests, 0 benchmarks\n");
        assert_eq!(
            r,
            TestListing {
                binary_count: 1,
                ..Default::default()
            }
        );
    }
}
