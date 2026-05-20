//! Spec §2.1 levels 4 (token stream) and 5 (comment stream).
//!
//! Level 4 uses `proc_macro2`'s standalone lexer so we can read tokens
//! without parsing into a full AST. Level 5 hand-scans the source text:
//! `proc_macro2` deliberately strips comments, so a separate pass over
//! the raw text is the only way to see them at this layer.

use std::collections::HashSet;
use std::str::FromStr;

use cstat_orchestrate::{Context, CostClass, Level, LevelError, LevelOutput};
use proc_macro2::{TokenStream, TokenTree};

use crate::inventory::SharedInventory;

// ───── Level 4: token stream ────────────────────────────────────────────

pub(crate) struct TokStreamLevel {
    pub shared: SharedInventory,
}

impl Level for TokStreamLevel {
    fn id(&self) -> &'static str { "tok.stream" }
    fn cost_class(&self) -> CostClass { CostClass::F }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let inv = self.shared.get(&ctx.project_root);

        let mut counts = TokenCounts::default();
        let mut unique_idents: HashSet<String> = HashSet::new();

        for f in &inv.rust_files {
            let Some(text) = &f.text else { continue };
            let Ok(stream) = TokenStream::from_str(text) else { continue };
            walk_tokens(stream, &mut counts, &mut unique_idents);
        }

        let density = if counts.total > 0 {
            counts.operators as f64 / counts.total as f64
        } else {
            0.0
        };

        let vector = vec![
            ("tok.global.token_count".into(), counts.total as f64),
            ("tok.global.identifier_count".into(), counts.identifiers as f64),
            ("tok.global.unique_identifiers".into(), unique_idents.len() as f64),
            ("tok.global.operator_count".into(), counts.operators as f64),
            ("tok.global.literal_count".into(), counts.literals as f64),
            ("tok.global.operator_density".into(), density),
            ("tok.global.raw_string_count".into(), counts.raw_strings as f64),
        ];

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

#[derive(Default)]
struct TokenCounts {
    total: u64,
    identifiers: u64,
    operators: u64,
    literals: u64,
    raw_strings: u64,
}

fn walk_tokens(stream: TokenStream, counts: &mut TokenCounts, unique: &mut HashSet<String>) {
    for tt in stream {
        counts.total += 1;
        match tt {
            TokenTree::Ident(i) => {
                counts.identifiers += 1;
                unique.insert(i.to_string());
            }
            TokenTree::Punct(_) => {
                counts.operators += 1;
            }
            TokenTree::Literal(l) => {
                counts.literals += 1;
                let s = l.to_string();
                if s.starts_with("r\"")
                    || s.starts_with("r#")
                    || s.starts_with("br\"")
                    || s.starts_with("br#")
                {
                    counts.raw_strings += 1;
                }
            }
            TokenTree::Group(g) => {
                walk_tokens(g.stream(), counts, unique);
            }
        }
    }
}

// ───── Level 5: comment stream ──────────────────────────────────────────

pub(crate) struct TokCommentsLevel {
    pub shared: SharedInventory,
}

impl Level for TokCommentsLevel {
    fn id(&self) -> &'static str { "tok.comments" }
    fn cost_class(&self) -> CostClass { CostClass::F }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let inv = self.shared.get(&ctx.project_root);

        let mut agg = CommentStats::default();
        for f in &inv.rust_files {
            if let Some(text) = &f.text {
                agg.combine(scan_comments(text));
            }
        }

        let density = if agg.total_lines > 0 {
            agg.comment_lines as f64 / agg.total_lines as f64
        } else {
            0.0
        };
        let doc_to_code = if agg.code_lines > 0 {
            agg.doc_comments as f64 / agg.code_lines as f64
        } else {
            0.0
        };

        let vector = vec![
            ("tok.comments.line_comment_count".into(), agg.line_comments as f64),
            ("tok.comments.block_comment_count".into(), agg.block_comments as f64),
            ("tok.comments.doc_comment_count".into(), agg.doc_comments as f64),
            ("tok.comments.todo_count".into(), agg.todos as f64),
            ("tok.comments.fixme_count".into(), agg.fixmes as f64),
            ("tok.comments.comment_density".into(), density),
            ("tok.comments.doc_to_code_ratio".into(), doc_to_code),
        ];

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

#[derive(Default)]
struct CommentStats {
    line_comments: u64,
    block_comments: u64,
    doc_comments: u64,
    todos: u64,
    fixmes: u64,
    comment_lines: u64,
    code_lines: u64,
    total_lines: u64,
}

impl CommentStats {
    fn combine(&mut self, other: CommentStats) {
        self.line_comments += other.line_comments;
        self.block_comments += other.block_comments;
        self.doc_comments += other.doc_comments;
        self.todos += other.todos;
        self.fixmes += other.fixmes;
        self.comment_lines += other.comment_lines;
        self.code_lines += other.code_lines;
        self.total_lines += other.total_lines;
    }
}

/// String-literal-naive comment scanner. Misses cases like `let s = "//"`
/// counting the `//` as a comment; that's intentional — this layer is
/// "source text observation," not a parser. The §2.2 AST level is where
/// we ask `syn` for ground-truth structure.
fn scan_comments(text: &str) -> CommentStats {
    let mut stats = CommentStats::default();
    let mut in_block = false;
    let mut block_is_doc = false;

    for line in text.lines() {
        stats.total_lines += 1;
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        let mut had_comment = false;
        let mut had_code = false;

        while i < chars.len() {
            if in_block {
                had_comment = true;
                if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                    in_block = false;
                    if block_is_doc {
                        stats.doc_comments += 1;
                    } else {
                        stats.block_comments += 1;
                    }
                    block_is_doc = false;
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }

            // Not currently in a block comment.
            if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
                had_comment = true;
                let is_doc = chars.get(i + 2).is_some_and(|c| *c == '/' || *c == '!');
                if is_doc {
                    stats.doc_comments += 1;
                } else {
                    stats.line_comments += 1;
                }
                let rest: String = chars[i..].iter().collect();
                if rest.contains("TODO") {
                    stats.todos += 1;
                }
                if rest.contains("FIXME") {
                    stats.fixmes += 1;
                }
                break;
            } else if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
                had_comment = true;
                in_block = true;
                // /** xxx */ is a doc comment; /**/ and /* xxx */ are plain.
                // /*! xxx */ is also a doc comment.
                let triple_star_then_slash =
                    chars.get(i + 2) == Some(&'*') && chars.get(i + 3) == Some(&'/');
                let starts_doc_star = chars.get(i + 2) == Some(&'*') && !triple_star_then_slash;
                let starts_doc_bang = chars.get(i + 2) == Some(&'!');
                block_is_doc = starts_doc_star || starts_doc_bang;
                i += 2;
            } else {
                if !chars[i].is_whitespace() {
                    had_code = true;
                }
                i += 1;
            }
        }

        if had_comment {
            stats.comment_lines += 1;
        }
        if had_code {
            stats.code_lines += 1;
        }
    }

    stats
}
