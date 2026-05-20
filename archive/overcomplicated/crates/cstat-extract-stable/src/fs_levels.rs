//! Spec §2.1 levels 1–3: filesystem layout, raw bytes, source text.
//!
//! All three share the same [`SharedInventory`] and therefore the same
//! single walk over the project tree.

use std::path::Path;

use cstat_orchestrate::{Context, CostClass, Level, LevelError, LevelOutput};

use crate::helpers::percentile_u64;
use crate::inventory::SharedInventory;

// ───── Level 1: filesystem layout ───────────────────────────────────────

pub(crate) struct FsLayoutLevel {
    pub shared: SharedInventory,
}

impl Level for FsLayoutLevel {
    fn id(&self) -> &'static str { "fs.layout" }
    fn cost_class(&self) -> CostClass { CostClass::F }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let inv = self.shared.get(&ctx.project_root);

        let mut vector = vec![
            ("fs.global.file_count".into(), inv.files.len() as f64),
            ("fs.global.dir_count".into(), inv.directories.len() as f64),
            ("fs.global.rust_file_count".into(), inv.rust_files.len() as f64),
            (
                "fs.global.max_dir_depth".into(),
                inv.files
                    .iter()
                    .map(|f| f.depth)
                    .chain(inv.directories.iter().map(|d| d.depth))
                    .max()
                    .unwrap_or(0) as f64,
            ),
        ];

        if !inv.files.is_empty() {
            let mut sizes: Vec<u64> = inv.files.iter().map(|f| f.size_bytes).collect();
            sizes.sort_unstable();
            let mean = sizes.iter().sum::<u64>() as f64 / sizes.len() as f64;
            vector.push(("fs.layout.file_size_bytes.mean".into(), mean));
            vector.push(("fs.layout.file_size_bytes.max".into(), *sizes.last().unwrap() as f64));
            vector.push(("fs.layout.file_size_bytes.p50".into(), percentile_u64(&sizes, 0.5)));
            vector.push(("fs.layout.file_size_bytes.p90".into(), percentile_u64(&sizes, 0.9)));
        }

        let snake_ratio = if inv.rust_files.is_empty() {
            0.0
        } else {
            let snake = inv
                .rust_files
                .iter()
                .filter(|r| is_snake_case_stem(&r.relative_path))
                .count();
            snake as f64 / inv.rust_files.len() as f64
        };
        vector.push(("fs.layout.snake_case_file_ratio".into(), snake_ratio));

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

fn is_snake_case_stem(path: &Path) -> bool {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    !stem.is_empty()
        && stem
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

// ───── Level 2: raw bytes ───────────────────────────────────────────────

pub(crate) struct FsBytesLevel {
    pub shared: SharedInventory,
}

impl Level for FsBytesLevel {
    fn id(&self) -> &'static str { "fs.bytes" }
    fn cost_class(&self) -> CostClass { CostClass::F }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let inv = self.shared.get(&ctx.project_root);

        let total_bytes: u64 = inv.rust_files.iter().map(|f| f.bytes.len() as u64).sum();

        let (mut crlf, mut lf_only, mut bom, mut utf8, mut non_utf8) = (0u64, 0u64, 0u64, 0u64, 0u64);
        for f in &inv.rust_files {
            if f.bytes.starts_with(b"\xEF\xBB\xBF") {
                bom += 1;
            }
            let has_crlf = f.bytes.windows(2).any(|w| w == b"\r\n");
            let has_lf = f.bytes.contains(&b'\n');
            if has_crlf {
                crlf += 1;
            } else if has_lf {
                lf_only += 1;
            }
            if f.text.is_some() {
                utf8 += 1;
            } else {
                non_utf8 += 1;
            }
        }

        let vector = vec![
            ("fs.global.total_bytes".into(), total_bytes as f64),
            ("fs.bytes.crlf_file_count".into(), crlf as f64),
            ("fs.bytes.lf_file_count".into(), lf_only as f64),
            ("fs.bytes.bom_file_count".into(), bom as f64),
            ("fs.bytes.utf8_file_count".into(), utf8 as f64),
            ("fs.bytes.non_utf8_file_count".into(), non_utf8 as f64),
        ];

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Level 3: source text ─────────────────────────────────────────────

pub(crate) struct FsSourceLevel {
    pub shared: SharedInventory,
}

impl Level for FsSourceLevel {
    fn id(&self) -> &'static str { "fs.source" }
    fn cost_class(&self) -> CostClass { CostClass::F }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let inv = self.shared.get(&ctx.project_root);

        let mut total_lines = 0u64;
        let mut total_blank = 0u64;
        let mut total_chars = 0u64;
        let mut max_line_len = 0u64;
        let mut per_file_blank_ratio: Vec<f64> = Vec::new();
        let mut per_file_lines: Vec<u64> = Vec::new();

        for f in &inv.rust_files {
            let Some(text) = &f.text else { continue };
            let mut line_count = 0u64;
            let mut blank = 0u64;
            for line in text.lines() {
                line_count += 1;
                if line.trim().is_empty() {
                    blank += 1;
                }
                let len = line.chars().count() as u64;
                if len > max_line_len {
                    max_line_len = len;
                }
            }
            total_lines += line_count;
            total_blank += blank;
            total_chars += text.chars().count() as u64;
            if line_count > 0 {
                per_file_blank_ratio.push(blank as f64 / line_count as f64);
                per_file_lines.push(line_count);
            }
        }

        let mut vector = vec![
            ("fs.source.line_count.sum".into(), total_lines as f64),
            ("fs.source.blank_line_count.sum".into(), total_blank as f64),
            ("fs.source.char_count.sum".into(), total_chars as f64),
            ("fs.source.max_line_length.max".into(), max_line_len as f64),
        ];

        if !per_file_blank_ratio.is_empty() {
            let mean_blank =
                per_file_blank_ratio.iter().sum::<f64>() / per_file_blank_ratio.len() as f64;
            vector.push(("fs.source.blank_line_ratio.mean".into(), mean_blank));
            let mean_lines =
                per_file_lines.iter().sum::<u64>() as f64 / per_file_lines.len() as f64;
            vector.push(("fs.source.line_count.mean".into(), mean_lines));
        }

        Ok(LevelOutput { vector, ..Default::default() })
    }
}
