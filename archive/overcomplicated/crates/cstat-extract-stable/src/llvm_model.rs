//! Shared LLVM-IR parse used by the Stage-14 derived levels.
//!
//! Stage 6's `parse_llvm_ir_text` is purposefully thin: it records
//! per-function instruction/basic-block/opcode counts and nothing else.
//! Stage 14's levels (33 mono call graph, 35 per-function data-flow) need
//! richer signal — each function's outgoing call targets and its set of
//! named SSA defs/uses. Doing that in the Stage-6 parser would bloat the
//! cheap path; doing it twice would walk every `.ll` twice. The shape
//! here is the deeper parse, lazily computed once and shared between the
//! two new levels.
//!
//! The parser operates on the optimized IR. Pre-opt is post-monomorphization
//! too (rustc runs mono before any LLVM pass), but the optimized form has
//! fewer wrapper / spill artifacts and is what level 33's "what actually
//! gets called after inlining" framing wants. Level 35's def-use chains
//! also live more cleanly in opt — after SROA/mem2reg, named values
//! correspond to real SSA defs rather than alloca slots.

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

use cstat_orchestrate::{Context, LevelError};

use crate::codegen_levels::SharedCodegen;

#[derive(Clone)]
pub(crate) struct SharedLlvmModel {
    codegen: SharedCodegen,
    inner: Arc<OnceLock<Result<LlvmModel, LlvmSkip>>>,
}

#[derive(Debug, Clone)]
struct LlvmSkip {
    reason: String,
    is_failure: bool,
}

impl LlvmSkip {
    fn into_level_error(self) -> LevelError {
        if self.is_failure {
            LevelError::Failed(self.reason)
        } else {
            LevelError::Skipped(self.reason)
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct LlvmModel {
    pub(crate) functions: Vec<IrFunctionDetail>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct IrFunctionDetail {
    pub name: String,
    pub instruction_count: u64,
    pub basic_block_count: u64,
    /// Symbol names of every direct call/invoke target in the body, in
    /// source order. Duplicates preserved so callsite counts are
    /// recoverable; level 33 dedups for the graph edge set.
    pub callees: Vec<String>,
    /// Named SSA defs introduced inside the body (i.e. the LHS of a
    /// `%name = ...` instruction). Block labels are NOT defs — they're
    /// control-flow targets, not data values, and don't participate in
    /// def-use chains.
    pub defs: BTreeSet<String>,
    /// Total def-use edges within this function: for each instruction
    /// that USES a name `%foo`, if `%foo` was defined earlier inside the
    /// same function, count one edge. References to function parameters
    /// or external values don't count — they have no in-function def
    /// site that the chain could anchor on.
    pub def_use_edge_count: u64,
    /// Total uses of named values inside this function (whether the def
    /// is local or not). Useful as a denominator for shape comparisons.
    pub use_count: u64,
}

impl SharedLlvmModel {
    pub(crate) fn new(codegen: SharedCodegen) -> Self {
        Self {
            codegen,
            inner: Arc::new(OnceLock::new()),
        }
    }

    pub(crate) fn get(&self, ctx: &Context) -> Result<&LlvmModel, LevelError> {
        let outcome = self.inner.get_or_init(|| build_model(&self.codegen, ctx));
        match outcome {
            Ok(model) => Ok(model),
            Err(skip) => Err(skip.clone().into_level_error()),
        }
    }
}

fn build_model(codegen: &SharedCodegen, ctx: &Context) -> Result<LlvmModel, LlvmSkip> {
    let cg = match codegen.get(ctx) {
        Ok(a) => a,
        Err(LevelError::Skipped(reason)) => {
            return Err(LlvmSkip { reason, is_failure: false });
        }
        Err(LevelError::Failed(reason)) => {
            return Err(LlvmSkip { reason, is_failure: true });
        }
    };
    let mut model = LlvmModel::default();
    for path in cg.opt_ir_files_view() {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        parse_llvm_ir_detailed(&text, &mut model.functions);
    }
    if model.functions.is_empty() {
        return Err(LlvmSkip {
            reason: "optimized LLVM IR contained no function definitions".into(),
            is_failure: true,
        });
    }
    Ok(model)
}

/// Walk one `.ll` file, accumulating per-function detail.
///
/// Per-function pass:
/// 1. On `define ... @name(...) { ... }` start a fresh accumulator.
/// 2. For each instruction line inside the body:
///    - If the LHS matches `%defname = ...`, record the def name.
///    - Scan the RHS (and operands) for `%use` tokens; for each, if the
///      use was previously defined in this function, increment the
///      def-use edge count.
///    - If the opcode is `call`/`invoke` and the call target starts
///      with `@`, capture the called symbol name.
/// 3. Block labels increment basic_block_count and don't participate in
///    defs/uses.
pub(crate) fn parse_llvm_ir_detailed(text: &str, out: &mut Vec<IrFunctionDetail>) {
    let mut cur: Option<IrFunctionDetail> = None;
    let mut defs_in_scope: BTreeSet<String> = BTreeSet::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if cur.is_none() {
            if let Some(name) = parse_define_function_name(line) {
                cur = Some(IrFunctionDetail { name, ..Default::default() });
                defs_in_scope.clear();
            }
            continue;
        }
        if line == "}" {
            let mut f = cur.take().expect("checked is_some above");
            f.defs = std::mem::take(&mut defs_in_scope);
            out.push(f);
            continue;
        }
        let f = cur.as_mut().expect("checked is_some above");
        let body = line.split(';').next().unwrap_or("").trim();
        if body.is_empty() {
            continue;
        }
        if is_ir_label_line(body) {
            f.basic_block_count += 1;
            continue;
        }
        // First implicit basic block gets a count on its first instruction.
        if f.basic_block_count == 0 {
            f.basic_block_count = 1;
        }
        f.instruction_count += 1;
        // Split LHS def from RHS so a `%defname = ...` line doesn't count
        // its own def as a use of itself.
        let (def, rhs) = split_def_from_rhs(body);
        if let Some(name) = def {
            defs_in_scope.insert(name);
        }
        // Count every `%foo`-shaped token on the RHS as a use; if the
        // name is locally defined, also add a def-use edge.
        for use_name in iter_value_uses(rhs) {
            f.use_count += 1;
            if defs_in_scope.contains(&use_name) {
                f.def_use_edge_count += 1;
            }
        }
        // Capture call/invoke targets. Operand position is opcode-specific;
        // for direct calls the target appears as `@<name>` after the type
        // signature on the RHS. The first `@`-prefixed bare token is the
        // callee. We're deliberately tolerant: indirect calls (target is
        // `%vtable_ptr`) produce no `@` here and are intentionally skipped
        // — level 33 is the static-after-mono graph, not the dynamic one.
        let opcode = extract_ir_opcode(rhs);
        if opcode == "call" || opcode == "invoke" {
            if let Some(target) = parse_direct_call_target(rhs) {
                f.callees.push(target);
            }
        }
    }
}

fn parse_define_function_name(line: &str) -> Option<String> {
    if !line.starts_with("define ") {
        return None;
    }
    let at = line.find('@')?;
    let rest = &line[at + 1..];
    let (name_end, _) = if rest.starts_with('"') {
        let after = &rest[1..];
        let end = after.find('"')?;
        (end + 2, end)
    } else {
        let stop = rest
            .find(|c: char| c == '(' || c == ' ' || c == '\t')
            .unwrap_or(rest.len());
        (stop, stop)
    };
    let name = if rest.starts_with('"') {
        rest[1..name_end - 1].to_string()
    } else {
        rest[..name_end].to_string()
    };
    if name.is_empty() { None } else { Some(name) }
}

fn is_ir_label_line(line: &str) -> bool {
    if !line.ends_with(':') {
        return false;
    }
    let name = &line[..line.len() - 1];
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// Split a `body` line into `(def_name, rhs)`. The def_name is the bare
/// `%name` SSA register on the LHS of an `=`, with the leading `%`
/// stripped. If the line has no `=` or the LHS isn't `%`-prefixed, def
/// is None and the whole line is the RHS.
fn split_def_from_rhs(body: &str) -> (Option<String>, &str) {
    let Some(eq) = body.find('=') else { return (None, body) };
    let lhs = body[..eq].trim();
    let rhs = body[eq + 1..].trim_start();
    if let Some(name) = lhs.strip_prefix('%') {
        let clean: String = name
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
            .collect();
        if clean.is_empty() {
            return (None, body);
        }
        return (Some(clean), rhs);
    }
    (None, body)
}

fn iter_value_uses(rhs: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = rhs.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() {
                let c = bytes[end];
                if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' {
                    end += 1;
                } else {
                    break;
                }
            }
            if end > start {
                out.push(rhs[start..end].to_string());
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

fn extract_ir_opcode(line: &str) -> String {
    // Same shape as the Stage-6 parser, kept local so a future tweak to
    // either side doesn't accidentally affect both.
    let after_eq = if let Some(idx) = line.find('=') {
        line[idx + 1..].trim()
    } else {
        line
    };
    let mut words = after_eq.split_whitespace();
    let first = clean_opcode_token(words.next().unwrap_or(""));
    if first.is_empty() {
        return String::new();
    }
    if first == "tail" || first == "musttail" || first == "notail" {
        return clean_opcode_token(words.next().unwrap_or(""));
    }
    first
}

fn clean_opcode_token(raw: &str) -> String {
    raw.chars()
        .take_while(|c| c.is_ascii_alphabetic() || *c == '_')
        .collect()
}

/// Parse the called symbol name out of a `call`/`invoke` instruction body.
/// Returns the bare symbol name (no leading `@`, no trailing parens),
/// or `None` for an indirect call (target is `%vptr` or a constant
/// expression rather than `@global`).
fn parse_direct_call_target(rhs: &str) -> Option<String> {
    let at = rhs.find('@')?;
    let after = &rhs[at + 1..];
    if after.starts_with('"') {
        let inner_end = after[1..].find('"')?;
        return Some(after[1..1 + inner_end].to_string());
    }
    let end = after
        .find(|c: char| c == '(' || c == ',' || c == ' ' || c == '\t')
        .unwrap_or(after.len());
    let name = after[..end].to_string();
    if name.is_empty() { None } else { Some(name) }
}

impl crate::codegen_levels::CodegenArtifacts {
    pub(crate) fn opt_ir_files_view(&self) -> &[std::path::PathBuf] {
        self.opt_ir_files_accessor()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_function_with_one_def_one_use() {
        let text = "\
define i32 @add(i32 %a, i32 %b) {
start:
  %s = add i32 %a, %b
  ret i32 %s
}
";
        let mut out = Vec::new();
        parse_llvm_ir_detailed(text, &mut out);
        assert_eq!(out.len(), 1);
        let f = &out[0];
        assert_eq!(f.name, "add");
        assert!(f.defs.contains("s"));
        assert_eq!(f.def_use_edge_count, 1, "ret reads %s once");
        assert!(f.use_count >= 3, "ret %s + add %a, %b");
    }

    #[test]
    fn counts_call_target_symbols() {
        let text = "\
define void @main() {
  %x = call i32 @foo(i32 0)
  call void @bar()
  ret void
}
";
        let mut out = Vec::new();
        parse_llvm_ir_detailed(text, &mut out);
        let f = out.into_iter().next().unwrap();
        assert_eq!(f.callees, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn ignores_indirect_calls_and_metadata() {
        let text = "\
define void @main() {
  %p = load ptr, ptr null
  call void %p()
  ret void
}
";
        let mut out = Vec::new();
        parse_llvm_ir_detailed(text, &mut out);
        let f = out.into_iter().next().unwrap();
        assert!(f.callees.is_empty(), "indirect call has no @-target");
    }

    #[test]
    fn def_use_only_counts_local_chain() {
        // %a is a function parameter, so a use of %a doesn't add an
        // edge — only locally-defined values anchor a chain.
        let text = "\
define i32 @f(i32 %a) {
  %b = add i32 %a, 1
  %c = mul i32 %b, %b
  ret i32 %c
}
";
        let mut out = Vec::new();
        parse_llvm_ir_detailed(text, &mut out);
        let f = out.into_iter().next().unwrap();
        // edges: %b used twice in mul, %c used once in ret → 3.
        assert_eq!(f.def_use_edge_count, 3);
    }

    #[test]
    fn skips_declarations_and_top_level_metadata() {
        let text = "\
target triple = \"x86_64-unknown-linux-gnu\"
declare void @ext(i32)
define void @body() {
  call void @ext(i32 0)
  ret void
}
!0 = !{i32 1}
";
        let mut out = Vec::new();
        parse_llvm_ir_detailed(text, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "body");
        assert_eq!(out[0].callees, vec!["ext".to_string()]);
    }
}
