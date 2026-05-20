//! Spec §2.1 level 6 (whitespace / formatting) and §2.9 level 47
//! (rustfmt diff size).

use std::process::Command;

use cstat_orchestrate::{Context, CostClass, Level, LevelError, LevelOutput};

use crate::helpers::percentile_u64;
use crate::inventory::SharedInventory;

// ───── Level 6: whitespace and formatting ───────────────────────────────

pub(crate) struct FmtWhitespaceLevel {
    pub shared: SharedInventory,
}

impl Level for FmtWhitespaceLevel {
    fn id(&self) -> &'static str { "fmt.whitespace" }
    fn cost_class(&self) -> CostClass { CostClass::F }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let inv = self.shared.get(&ctx.project_root);

        let mut tab_files = 0u64;
        let mut space_files = 0u64;
        let mut mixed_files = 0u64;
        let mut trailing_ws_lines = 0u64;
        let mut line_lengths: Vec<u64> = Vec::new();

        for f in &inv.rust_files {
            let Some(text) = &f.text else { continue };
            let mut saw_tab_indent = false;
            let mut saw_space_indent = false;
            for line in text.lines() {
                let mut indent_chars = line.chars().take_while(|c| c.is_whitespace());
                let mut had_indent_tab = false;
                let mut had_indent_space = false;
                for c in indent_chars.by_ref() {
                    if c == '\t' {
                        had_indent_tab = true;
                    } else if c == ' ' {
                        had_indent_space = true;
                    }
                }
                if had_indent_tab {
                    saw_tab_indent = true;
                }
                if had_indent_space && line.trim_start().len() < line.len() {
                    // Only count space indent if there is actual non-whitespace content;
                    // a fully-blank line is not a vote for either style.
                    saw_space_indent = true;
                }
                line_lengths.push(line.chars().count() as u64);
                if line.len() > line.trim_end().len() {
                    trailing_ws_lines += 1;
                }
            }
            match (saw_tab_indent, saw_space_indent) {
                (true, false) => tab_files += 1,
                (false, true) => space_files += 1,
                (true, true) => mixed_files += 1,
                _ => {}
            }
        }

        let mut vector = vec![
            ("fmt.whitespace.tab_indented_file_count".into(), tab_files as f64),
            ("fmt.whitespace.space_indented_file_count".into(), space_files as f64),
            ("fmt.whitespace.mixed_indented_file_count".into(), mixed_files as f64),
            ("fmt.whitespace.trailing_whitespace_line_count".into(), trailing_ws_lines as f64),
        ];

        if !line_lengths.is_empty() {
            line_lengths.sort_unstable();
            let mean = line_lengths.iter().sum::<u64>() as f64 / line_lengths.len() as f64;
            vector.push(("fmt.whitespace.line_length.mean".into(), mean));
            vector.push(("fmt.whitespace.line_length.max".into(), *line_lengths.last().unwrap() as f64));
            vector.push(("fmt.whitespace.line_length.p50".into(), percentile_u64(&line_lengths, 0.5)));
            vector.push(("fmt.whitespace.line_length.p90".into(), percentile_u64(&line_lengths, 0.9)));
        }

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Level 47: rustfmt diff ───────────────────────────────────────────

pub(crate) struct RustfmtLevel;

impl Level for RustfmtLevel {
    fn id(&self) -> &'static str { "rustfmt" }
    fn cost_class(&self) -> CostClass { CostClass::F }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let output = Command::new("cargo")
            .arg("fmt")
            .arg("--all")
            .arg("--check")
            .current_dir(&ctx.project_root)
            .output()
            .map_err(|e| LevelError::Skipped(format!("cargo not available: {e}")))?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // rustfmt exits 0 when conformant, 1 when it would change things,
        // anything else is a real error (missing component, parse failure, etc.).
        if exit_code != 0 && exit_code != 1 {
            let msg = stderr.trim();
            if msg.contains("rustfmt") && (msg.contains("not installed") || msg.contains("component"))
            {
                return Err(LevelError::Skipped(format!("rustfmt unavailable: {msg}")));
            }
            return Err(LevelError::Failed(format!(
                "cargo fmt --check exited {exit_code}: {msg}"
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let diff_line_count = stdout
            .lines()
            .filter(|l| {
                (l.starts_with('+') || l.starts_with('-'))
                    && !l.starts_with("+++")
                    && !l.starts_with("---")
            })
            .count() as f64;
        let files_needing = stdout
            .lines()
            .filter(|l| l.starts_with("Diff in "))
            .count() as f64;

        let vector = vec![
            ("rustfmt.conformant".into(), if exit_code == 0 { 1.0 } else { 0.0 }),
            ("rustfmt.diff_line_count".into(), diff_line_count),
            ("rustfmt.files_needing_fmt".into(), files_needing),
        ];

        Ok(LevelOutput { vector, ..Default::default() })
    }
}
