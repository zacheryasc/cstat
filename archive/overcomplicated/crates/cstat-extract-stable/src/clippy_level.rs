//! Spec §2.9 level 45 — clippy lint inventory.
//!
//! Drives `cargo clippy --message-format=json` from inside the project
//! root with a private target directory so we don't fight other cargo
//! invocations for build locks. Each line of clippy's stream is one
//! JSON message; `compiler-message` entries carry the lint diagnostics
//! we need. Anything else (build artifacts, build-script output) is
//! ignored.
//!
//! Skips with a reason when cargo or the clippy component is missing.
//! Surfaces a Failed if the cargo invocation itself blew up (e.g. the
//! project doesn't build) — clippy-the-tool succeeding with lints is
//! still ok, since clippy reports warnings via diagnostics, not exit
//! code.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use cstat_orchestrate::{Context, CostClass, Level, LevelError, LevelOutput};
use serde::Deserialize;

pub(crate) struct ClippyLevel;

impl Level for ClippyLevel {
    fn id(&self) -> &'static str { "clippy" }
    fn cost_class(&self) -> CostClass { CostClass::B }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let target_dir = ctx.project_root.join("target").join("cstat-clippy");
        let output = run_clippy(&ctx.project_root, &target_dir)
            .map_err(|e| classify_clippy_error(&e))?;

        let summary = tally_messages(&output.stdout);

        let vector = vec![
            ("clippy.message_count".into(), summary.message_count as f64),
            ("clippy.lint_count".into(), summary.lint_count as f64),
            ("clippy.warning_count".into(), summary.warning_count as f64),
            ("clippy.error_count".into(), summary.error_count as f64),
            ("clippy.note_count".into(), summary.note_count as f64),
            ("clippy.help_count".into(), summary.help_count as f64),
            (
                "clippy.distinct_lint_names".into(),
                summary.distinct_lint_names as f64,
            ),
            (
                "clippy.files_with_lints".into(),
                summary.files_with_lints as f64,
            ),
        ];

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

struct ClippyOutput {
    stdout: Vec<u8>,
}

fn run_clippy(project_root: &Path, target_dir: &Path) -> Result<ClippyOutput, String> {
    let output = Command::new("cargo")
        .arg("clippy")
        .arg("--all-targets")
        .arg("--message-format=json")
        .arg("--quiet")
        .arg("--target-dir")
        .arg(target_dir)
        .current_dir(project_root)
        .output()
        .map_err(|e| format!("spawn cargo clippy: {e}"))?;

    // Clippy lints don't change the exit code: a clean compile that
    // raised 200 `warn`s still exits 0. A non-zero exit means the
    // build itself broke or clippy isn't installed — both are
    // problems we should surface, not silently treat as "no lints".
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "exit {:?}: {}",
            output.status.code(),
            stderr.lines().rev().take(8).collect::<Vec<_>>().join(" | ")
        ));
    }

    Ok(ClippyOutput { stdout: output.stdout })
}

fn classify_clippy_error(msg: &str) -> LevelError {
    let lc = msg.to_ascii_lowercase();
    // "no such command: `clippy`" / "the 'clippy' component is not installed"
    if lc.contains("no such command")
        || lc.contains("not installed")
        || lc.contains("clippy is not")
        || lc.contains("component")
    {
        return LevelError::Skipped(format!("clippy unavailable: {msg}"));
    }
    if lc.contains("spawn cargo") {
        return LevelError::Skipped(format!("cargo not available: {msg}"));
    }
    LevelError::Failed(format!("cargo clippy: {msg}"))
}

#[derive(Default, Debug)]
struct LintSummary {
    message_count: u64,
    lint_count: u64,
    warning_count: u64,
    error_count: u64,
    note_count: u64,
    help_count: u64,
    distinct_lint_names: u64,
    files_with_lints: u64,
}

fn tally_messages(stdout: &[u8]) -> LintSummary {
    let mut distinct_lints: BTreeSet<String> = BTreeSet::new();
    let mut files: BTreeSet<String> = BTreeSet::new();
    let mut summary = LintSummary::default();

    for line in stdout.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let envelope: CargoMessage = match serde_json::from_slice(line) {
            Ok(v) => v,
            // Build output occasionally contains non-JSON noise on
            // stderr-merged streams; we only read stdout so this is
            // rare. Skip silently rather than failing the level.
            Err(_) => continue,
        };
        if envelope.reason.as_deref() != Some("compiler-message") {
            continue;
        }
        let Some(message) = envelope.message else { continue };
        summary.message_count += 1;
        if let Some(level) = message.level.as_deref() {
            match level {
                "warning" => summary.warning_count += 1,
                "error" | "error: internal compiler error" => summary.error_count += 1,
                "note" => summary.note_count += 1,
                "help" => summary.help_count += 1,
                _ => {}
            }
        }
        if let Some(code) = message.code.and_then(|c| c.code) {
            // Bare rustc diagnostics show up too (e.g. "unused_imports").
            // Both rustc and clippy lint names are reported here; we
            // tally them together as "lints" since the level's spec is
            // "lint inventory".
            summary.lint_count += 1;
            distinct_lints.insert(code);
        }
        for span in message.spans.unwrap_or_default() {
            if let Some(file) = span.file_name {
                files.insert(file);
            }
        }
    }

    summary.distinct_lint_names = distinct_lints.len() as u64;
    summary.files_with_lints = files.len() as u64;
    summary
}

#[derive(Deserialize)]
struct CargoMessage {
    reason: Option<String>,
    message: Option<DiagnosticMessage>,
}

#[derive(Deserialize)]
struct DiagnosticMessage {
    level: Option<String>,
    code: Option<DiagnosticCode>,
    spans: Option<Vec<DiagnosticSpan>>,
}

#[derive(Deserialize)]
struct DiagnosticCode {
    code: Option<String>,
}

#[derive(Deserialize)]
struct DiagnosticSpan {
    file_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tally_counts_warnings_and_distinct_lint_names() {
        let stream = br#"{"reason":"compiler-artifact"}
{"reason":"compiler-message","message":{"level":"warning","code":{"code":"clippy::needless_return"},"spans":[{"file_name":"src/lib.rs"}]}}
{"reason":"compiler-message","message":{"level":"warning","code":{"code":"clippy::needless_return"},"spans":[{"file_name":"src/lib.rs"}]}}
{"reason":"compiler-message","message":{"level":"warning","code":{"code":"clippy::let_unit_value"},"spans":[{"file_name":"src/other.rs"}]}}
{"reason":"build-finished","success":true}
"#;
        let s = tally_messages(stream);
        assert_eq!(s.message_count, 3);
        assert_eq!(s.warning_count, 3);
        assert_eq!(s.lint_count, 3);
        assert_eq!(s.distinct_lint_names, 2);
        assert_eq!(s.files_with_lints, 2);
        assert_eq!(s.error_count, 0);
    }

    #[test]
    fn tally_ignores_messages_with_no_code() {
        // Plain compiler messages without a lint code (e.g. a bare
        // "note" attached to a top-level warning) should bump the
        // message_count but not the lint_count.
        let stream = br#"{"reason":"compiler-message","message":{"level":"note","spans":[]}}
"#;
        let s = tally_messages(stream);
        assert_eq!(s.message_count, 1);
        assert_eq!(s.note_count, 1);
        assert_eq!(s.lint_count, 0);
        assert_eq!(s.distinct_lint_names, 0);
    }

    #[test]
    fn classify_recognizes_missing_clippy_component() {
        let err = classify_clippy_error("no such command: `clippy`");
        assert!(matches!(err, LevelError::Skipped(_)));
        let err = classify_clippy_error("error: the 'clippy' component is not installed");
        assert!(matches!(err, LevelError::Skipped(_)));
    }

    #[test]
    fn classify_treats_build_failure_as_failed() {
        let err = classify_clippy_error("exit Some(101): error[E0425]: cannot find value `x`");
        assert!(matches!(err, LevelError::Failed(_)));
    }
}
