//! Spec §2.5 levels 20–24: codegen extractors.
//!
//! These are the first stable-side levels that pay a real build cost
//! (`recompute_cost_class = C`). They drive `cargo rustc` twice — once
//! with `--emit=llvm-ir -C no-prepopulate-passes` for the pre-opt IR
//! (level 20), once with `--emit=llvm-ir,asm,link` for the optimized
//! IR (21), assembly (22), object code (23), and linked artifact (24).
//!
//! Off by default: levels skip with reason "codegen disabled" unless
//! the caller passes `--enable-codegen`. The dev-plan rationale is
//! that codegen and rustc subprocesses dominate wall-clock time; the
//! gate keeps the default extraction fast.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

use cstat_orchestrate::{Context, CostClass, EntityRecord, Level, LevelError, LevelOutput};
use cstat_schema::IdSet;

/// Lazy, shared cache of cargo-built codegen artifacts. The first level
/// whose `extract` runs triggers the two cargo builds; subsequent
/// levels reuse the cached paths.
#[derive(Clone)]
pub(crate) struct SharedCodegen {
    inner: Arc<OnceLock<CodegenResult>>,
}

type CodegenResult = Result<CodegenArtifacts, CodegenSkip>;

/// Why the codegen cache could not be populated. The five levels
/// translate this into either `Skipped` (`disabled`/`unsupported`) or
/// `Failed` (a real build error).
#[derive(Debug, Clone)]
struct CodegenSkip {
    reason: String,
    is_failure: bool,
}

impl CodegenSkip {
    fn into_level_error(self) -> LevelError {
        if self.is_failure {
            LevelError::Failed(self.reason)
        } else {
            LevelError::Skipped(self.reason)
        }
    }

    fn skipped(reason: impl Into<String>) -> Self {
        Self { reason: reason.into(), is_failure: false }
    }
    fn failed(reason: impl Into<String>) -> Self {
        Self { reason: reason.into(), is_failure: true }
    }
}

/// Files cargo produced for us. Each list may be empty if the project
/// has no target that produces that kind of artifact.
#[derive(Debug)]
pub(crate) struct CodegenArtifacts {
    preopt_ir_files: Vec<PathBuf>,
    opt_ir_files: Vec<PathBuf>,
    asm_files: Vec<PathBuf>,
    object_files: Vec<PathBuf>,
    /// `.rlib` / static library artifacts.
    rlib_files: Vec<PathBuf>,
    /// Linked executable, if the crate has a binary target.
    binary_file: Option<PathBuf>,
}

impl CodegenArtifacts {
    pub(crate) fn object_files_view(&self) -> &[PathBuf] {
        &self.object_files
    }
    pub(crate) fn rlib_files_view(&self) -> &[PathBuf] {
        &self.rlib_files
    }
    pub(crate) fn opt_ir_files_accessor(&self) -> &[PathBuf] {
        &self.opt_ir_files
    }
}

impl SharedCodegen {
    pub(crate) fn new() -> Self {
        Self { inner: Arc::new(OnceLock::new()) }
    }

    pub(crate) fn get(&self, ctx: &Context) -> Result<&CodegenArtifacts, LevelError> {
        let result = self.inner.get_or_init(|| build_codegen_artifacts(ctx));
        match result {
            Ok(a) => Ok(a),
            Err(skip) => Err(skip.clone().into_level_error()),
        }
    }
}

fn build_codegen_artifacts(ctx: &Context) -> CodegenResult {
    if !ctx.config.enable_codegen {
        return Err(CodegenSkip::skipped(
            "codegen disabled (pass --enable-codegen to opt in; cost class C)",
        ));
    }

    // Cargo must be on PATH.
    let cargo_probe = Command::new("cargo").arg("--version").output();
    if let Err(e) = cargo_probe {
        return Err(CodegenSkip::skipped(format!("cargo not available: {e}")));
    }

    // Virtual workspaces (no [package] at the root) are out of scope in
    // v1 — `cargo rustc` has no notion of "the package" without `-p`.
    let manifest_text = std::fs::read_to_string(ctx.project_root.join("Cargo.toml"))
        .map_err(|e| CodegenSkip::failed(format!("read Cargo.toml: {e}")))?;
    let manifest: toml::Value = manifest_text
        .parse()
        .map_err(|e| CodegenSkip::failed(format!("parse Cargo.toml: {e}")))?;
    let root_is_package = manifest.get("package").is_some();
    if !root_is_package {
        return Err(CodegenSkip::skipped(
            "codegen on virtual workspaces is not implemented (v1 limitation; \
             run cstat with --enable-codegen from a member crate instead)",
        ));
    }

    // Build 1: pre-opt IR. Lives under a private target-dir so it
    // doesn't fight with build 2 over the same .ll filenames. We use
    // the dev profile (no `--release`) for two reasons: it follows the
    // spec §2.5 command literally, and the release profile aggressively
    // dead-code-strips unused pub items from rlibs, so for library
    // crates with no downstream callers the .ll file would be empty.
    let preopt_target = ctx.project_root.join("target").join("cstat-preopt");
    run_cargo_rustc(
        &ctx.project_root,
        &preopt_target,
        &["--emit=llvm-ir", "-C", "no-prepopulate-passes"],
    )
    .map_err(|e| CodegenSkip::failed(format!("pre-opt cargo rustc: {e}")))?;

    // Build 2: optimized IR + asm + the normal link output.
    let opt_target = ctx.project_root.join("target").join("cstat-opt");
    run_cargo_rustc(
        &ctx.project_root,
        &opt_target,
        &["--emit=llvm-ir,asm,link"],
    )
    .map_err(|e| CodegenSkip::failed(format!("optimized cargo rustc: {e}")))?;

    let preopt_deps = preopt_target.join("debug").join("deps");
    let opt_deps = opt_target.join("debug").join("deps");
    let opt_release = opt_target.join("debug");

    let preopt_ir_files = collect_by_extension(&preopt_deps, "ll");
    let opt_ir_files = collect_by_extension(&opt_deps, "ll");
    let asm_files = collect_by_extension(&opt_deps, "s");
    let object_files = collect_by_extension(&opt_deps, "o");
    let rlib_files = collect_by_extension(&opt_deps, "rlib");
    let binary_file = locate_binary_artifact(&opt_release);

    Ok(CodegenArtifacts {
        preopt_ir_files,
        opt_ir_files,
        asm_files,
        object_files,
        rlib_files,
        binary_file,
    })
}

/// Drive `cargo rustc` (dev profile) with the given extra rustc args.
///
/// Tries `--lib` first; on "no library targets" output, retries
/// without `--lib` so bin-only crates still work. Returns Err with
/// the stderr tail if both attempts fail.
fn run_cargo_rustc(project_root: &Path, target_dir: &Path, rustc_args: &[&str]) -> Result<(), String> {
    let attempts: [&[&str]; 2] = [&["--lib"], &[]];
    let mut last_err = String::new();
    for attempt in attempts {
        let mut cmd = Command::new("cargo");
        cmd.arg("rustc");
        for a in attempt {
            cmd.arg(a);
        }
        cmd.arg("--target-dir").arg(target_dir);
        cmd.arg("--").args(rustc_args);
        cmd.current_dir(project_root);
        let output = cmd
            .output()
            .map_err(|e| format!("failed to spawn cargo: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Heuristic: if the failure is "no library targets", fall through
        // to the next attempt. Otherwise return the error now.
        if !stderr.contains("no library targets") && !stderr.contains("no lib target") {
            return Err(format!(
                "exited {:?}: {}",
                output.status.code(),
                stderr.lines().rev().take(6).collect::<Vec<_>>().join(" | ")
            ));
        }
        last_err = stderr.into_owned();
    }
    Err(format!(
        "all cargo rustc attempts failed: {}",
        last_err.lines().rev().take(4).collect::<Vec<_>>().join(" | ")
    ))
}

fn collect_by_extension(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension() == Some(OsStr::new(ext)))
        .collect();
    out.sort();
    out
}

fn locate_binary_artifact(release_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(release_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        // Heuristic: skip .rlib, .rmeta, .d artifacts; any other regular
        // file with the executable bit set under target/release is our
        // best guess at "the linked binary." cargo writes binaries here
        // directly (not under deps/), so the filter is sharp.
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if matches!(ext, "rlib" | "rmeta" | "d" | "json" | "ll" | "s" | "o") {
            continue;
        }
        if let Ok(meta) = path.metadata() {
            if is_executable(&meta) {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    !meta.is_dir()
}

// ───── Level 20: pre-opt LLVM IR ────────────────────────────────────────

pub(crate) struct LlvmPreoptLevel {
    pub shared: SharedCodegen,
}

impl Level for LlvmPreoptLevel {
    fn id(&self) -> &'static str { "llvm.preopt" }
    fn cost_class(&self) -> CostClass { CostClass::C }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let artifacts = self.shared.get(ctx)?;
        let funcs = parse_all_ir(&artifacts.preopt_ir_files);
        if funcs.is_empty() {
            return Err(LevelError::Failed(
                "pre-opt build produced no .ll files (rustc emitted nothing)".into(),
            ));
        }
        let mut vector = ir_summary_vector("llvm.preopt", &funcs);
        vector.push((
            "llvm.preopt.module_count".into(),
            artifacts.preopt_ir_files.len() as f64,
        ));
        let entities = ir_function_entities("preopt", &funcs);
        Ok(LevelOutput { vector, entities })
    }
}

// ───── Level 21: optimized LLVM IR ──────────────────────────────────────

pub(crate) struct LlvmOptLevel {
    pub shared: SharedCodegen,
}

impl Level for LlvmOptLevel {
    fn id(&self) -> &'static str { "llvm.opt" }
    fn cost_class(&self) -> CostClass { CostClass::C }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let artifacts = self.shared.get(ctx)?;
        let funcs = parse_all_ir(&artifacts.opt_ir_files);
        if funcs.is_empty() {
            return Err(LevelError::Failed(
                "optimized build produced no .ll files (rustc emitted nothing)".into(),
            ));
        }
        let mut vector = ir_summary_vector("llvm.opt", &funcs);
        vector.push((
            "llvm.opt.module_count".into(),
            artifacts.opt_ir_files.len() as f64,
        ));
        // Inlining proxy: how many fewer functions does the optimizer
        // retain vs the pre-opt build? Negative = more (inlining
        // introducing wrappers / cgu fragmentation differing across
        // builds). Defended as a *proxy*, not a precise count.
        let preopt_funcs = parse_all_ir(&artifacts.preopt_ir_files);
        let preopt_n = preopt_funcs.len() as f64;
        let opt_n = funcs.len() as f64;
        let inline_ratio = if preopt_n > 0.0 {
            (preopt_n - opt_n).max(0.0) / preopt_n
        } else {
            0.0
        };
        vector.push(("llvm.opt.inline_ratio_proxy".into(), inline_ratio));
        let entities = ir_function_entities("opt", &funcs);
        Ok(LevelOutput { vector, entities })
    }
}

// ───── Level 22: assembly ───────────────────────────────────────────────

pub(crate) struct AsmLevel {
    pub shared: SharedCodegen,
}

impl Level for AsmLevel {
    fn id(&self) -> &'static str { "asm" }
    fn cost_class(&self) -> CostClass { CostClass::C }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let artifacts = self.shared.get(ctx)?;
        if artifacts.asm_files.is_empty() {
            return Err(LevelError::Failed(
                "optimized build produced no .s files (rustc emitted nothing)".into(),
            ));
        }
        let mut total_lines = 0u64;
        let mut total_instr = 0u64;
        let mut total_directives = 0u64;
        let mut total_funcs = 0u64;
        let mut total_bytes = 0u64;
        for path in &artifacts.asm_files {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            total_bytes += text.len() as u64;
            for line in text.lines() {
                total_lines += 1;
                let stripped = line.trim();
                if stripped.is_empty() || stripped.starts_with('#') || stripped.starts_with("//") {
                    continue;
                }
                if stripped.starts_with('.') {
                    total_directives += 1;
                    // Function declarations show up as ".type foo,@function"
                    if stripped.starts_with(".type") && stripped.contains("@function") {
                        total_funcs += 1;
                    }
                } else if is_asm_instruction_line(line) {
                    total_instr += 1;
                }
            }
        }
        let vector = vec![
            ("asm.global.file_count".into(), artifacts.asm_files.len() as f64),
            ("asm.global.line_count".into(), total_lines as f64),
            ("asm.global.instruction_line_count".into(), total_instr as f64),
            ("asm.global.directive_count".into(), total_directives as f64),
            ("asm.global.function_count".into(), total_funcs as f64),
            ("asm.global.total_bytes".into(), total_bytes as f64),
        ];
        Ok(LevelOutput { vector, ..Default::default() })
    }
}

fn is_asm_instruction_line(line: &str) -> bool {
    // Asm files written by rustc indent instruction mnemonics with a
    // tab; labels start at column 0 and end with ':'. This is enough
    // signal to count "instruction lines" without parsing every dialect.
    if !line.starts_with('\t') && !line.starts_with("    ") {
        return false;
    }
    let stripped = line.trim_start();
    if stripped.is_empty() || stripped.starts_with('.') || stripped.starts_with('#') {
        return false;
    }
    // Comment-only lines (e.g. "# -- Begin function") may also start
    // with whitespace; rule them out.
    let first = stripped.split_whitespace().next().unwrap_or("");
    !first.is_empty()
        && first
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
}

// ───── Level 23: object files ───────────────────────────────────────────

pub(crate) struct ObjLevel {
    pub shared: SharedCodegen,
}

impl Level for ObjLevel {
    fn id(&self) -> &'static str { "obj" }
    fn cost_class(&self) -> CostClass { CostClass::C }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let artifacts = self.shared.get(ctx)?;
        // For lib crates rustc emits an .rlib (an `ar` archive of one
        // or more .o files). For bin/cdylib targets we also get raw .o
        // files in deps/. Stage 6's metric is intentionally coarse:
        // counts + total bytes. Stage 7 splits the rlib into per-object
        // section sizes via the `object` crate.
        let mut total_bytes = 0u64;
        let mut object_count = 0u64;
        for p in &artifacts.object_files {
            if let Ok(m) = p.metadata() {
                total_bytes += m.len();
                object_count += 1;
            }
        }
        let mut rlib_bytes = 0u64;
        let mut rlib_count = 0u64;
        for p in &artifacts.rlib_files {
            if let Ok(m) = p.metadata() {
                rlib_bytes += m.len();
                rlib_count += 1;
            }
        }
        if object_count == 0 && rlib_count == 0 {
            return Err(LevelError::Failed(
                "optimized build produced no .o or .rlib artifacts".into(),
            ));
        }
        let vector = vec![
            ("obj.global.object_file_count".into(), object_count as f64),
            ("obj.global.object_total_bytes".into(), total_bytes as f64),
            ("obj.global.rlib_file_count".into(), rlib_count as f64),
            ("obj.global.rlib_total_bytes".into(), rlib_bytes as f64),
        ];
        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Level 24: linked binary ──────────────────────────────────────────

pub(crate) struct BinLevel {
    pub shared: SharedCodegen,
}

impl Level for BinLevel {
    fn id(&self) -> &'static str { "bin" }
    fn cost_class(&self) -> CostClass { CostClass::C }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let artifacts = self.shared.get(ctx)?;
        let Some(path) = &artifacts.binary_file else {
            return Err(LevelError::Skipped(
                "no [[bin]] target in the crate; nothing was linked".into(),
            ));
        };
        let meta = path
            .metadata()
            .map_err(|e| LevelError::Failed(format!("stat {path:?}: {e}")))?;
        let size = meta.len();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let name_len = name.len() as f64;
        let vector = vec![
            ("bin.size_bytes".into(), size as f64),
            ("bin.target_name_length".into(), name_len),
        ];
        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── LLVM IR text parser ──────────────────────────────────────────────

#[derive(Debug, Default)]
struct IrFunction {
    name: String,
    basic_block_count: u64,
    instruction_count: u64,
    call_site_count: u64,
    /// Histogram of opcode → count. Bucket count is bounded by the LLVM
    /// opcode set; on a Rust crate this stays small (~30 buckets).
    opcode_histogram: BTreeMap<String, u64>,
}

fn parse_all_ir(paths: &[PathBuf]) -> Vec<IrFunction> {
    let mut out = Vec::new();
    for p in paths {
        if let Ok(text) = std::fs::read_to_string(p) {
            out.extend(parse_llvm_ir_text(&text));
        }
    }
    out
}

/// Walk LLVM IR text, returning one [`IrFunction`] per `define ... { ... }`
/// block. Top-level constructs (declares, attributes, metadata, debug
/// nodes) are ignored — the per-function metrics are computed strictly
/// from each function body.
fn parse_llvm_ir_text(text: &str) -> Vec<IrFunction> {
    let mut funcs = Vec::new();
    let mut cur: Option<IrFunction> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if cur.is_none() {
            if let Some(name) = parse_define_function_name(line) {
                cur = Some(IrFunction { name, ..Default::default() });
            }
            continue;
        }
        if line == "}" {
            if let Some(f) = cur.take() {
                funcs.push(f);
            }
            continue;
        }
        let f = cur.as_mut().expect("checked is_some above");
        // Strip any inline ';' comment for opcode parsing.
        let body = line.split(';').next().unwrap_or("").trim();
        if body.is_empty() {
            continue;
        }
        if is_ir_label_line(body) {
            f.basic_block_count += 1;
            continue;
        }
        let opcode = extract_ir_opcode(body);
        if opcode.is_empty() {
            continue;
        }
        // The first basic block may be implicit (no leading label), in
        // which case the first instruction we see is the start of it.
        if f.basic_block_count == 0 {
            f.basic_block_count = 1;
        }
        f.instruction_count += 1;
        *f.opcode_histogram.entry(opcode.clone()).or_insert(0) += 1;
        if opcode == "call" || opcode == "invoke" {
            f.call_site_count += 1;
        }
    }
    funcs
}

fn parse_define_function_name(line: &str) -> Option<String> {
    if !line.starts_with("define ") {
        return None;
    }
    let at = line.find('@')?;
    let rest = &line[at + 1..];
    // Function symbols may be quoted: define ... @"name"(...) or bare.
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
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn is_ir_label_line(line: &str) -> bool {
    if !line.ends_with(':') {
        return false;
    }
    let name = &line[..line.len() - 1];
    if name.is_empty() {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

fn extract_ir_opcode(line: &str) -> String {
    // "%0 = call ..." → take the word after "=".
    // "ret i32 ..." / "br label %x" / "store ..." → first word.
    // "tail call ..." → second word ("call").
    let after_eq = if let Some(idx) = line.find('=') {
        line[idx + 1..].trim()
    } else {
        line
    };
    let mut words = after_eq.split_whitespace();
    let raw_first = words.next().unwrap_or("");
    let first = clean_opcode_token(raw_first);
    if first.is_empty() {
        return String::new();
    }
    if first == "tail" || first == "musttail" || first == "notail" {
        return clean_opcode_token(words.next().unwrap_or(""));
    }
    // Skip metadata / debug intrinsics — these start with '!' or '#'
    // and aren't real instructions in the cost-per-function sense.
    if first.starts_with('!') || first.starts_with('@') || first.starts_with('#') {
        return String::new();
    }
    first
}

/// Trim trailing punctuation/parens that appear when the first token of
/// an instruction is glued to following structure (e.g. `unreachable,`
/// or `#dbg_declare(ptr`). Only ASCII alphabetic + underscore are kept;
/// the result is the bare opcode mnemonic.
fn clean_opcode_token(raw: &str) -> String {
    let trimmed: String = raw
        .chars()
        .take_while(|c| c.is_ascii_alphabetic() || *c == '_')
        .collect();
    trimmed
}

fn ir_summary_vector(prefix: &str, funcs: &[IrFunction]) -> Vec<(String, f64)> {
    let n = funcs.len() as f64;
    let mut instr_total = 0u64;
    let mut bb_total = 0u64;
    let mut call_total = 0u64;
    let mut instr_max = 0u64;
    let mut bb_max = 0u64;
    for f in funcs {
        instr_total += f.instruction_count;
        bb_total += f.basic_block_count;
        call_total += f.call_site_count;
        if f.instruction_count > instr_max {
            instr_max = f.instruction_count;
        }
        if f.basic_block_count > bb_max {
            bb_max = f.basic_block_count;
        }
    }
    let instr_mean = if n > 0.0 { instr_total as f64 / n } else { 0.0 };
    let bb_mean = if n > 0.0 { bb_total as f64 / n } else { 0.0 };
    vec![
        (format!("{prefix}.function_count"), n),
        (format!("{prefix}.instruction_count.sum"), instr_total as f64),
        (format!("{prefix}.instruction_count.mean"), instr_mean),
        (format!("{prefix}.instruction_count.max"), instr_max as f64),
        (format!("{prefix}.basic_block_count.sum"), bb_total as f64),
        (format!("{prefix}.basic_block_count.mean"), bb_mean),
        (format!("{prefix}.basic_block_count.max"), bb_max as f64),
        (format!("{prefix}.call_site_count.sum"), call_total as f64),
    ]
}

fn ir_function_entities(
    variant: &str,
    funcs: &[IrFunction],
) -> BTreeMap<String, Vec<EntityRecord>> {
    let mut records = Vec::with_capacity(funcs.len());
    for f in funcs {
        let mut metrics = BTreeMap::new();
        metrics.insert(format!("{variant}_instructions"), f.instruction_count as f64);
        metrics.insert(format!("{variant}_basic_blocks"), f.basic_block_count as f64);
        metrics.insert(format!("{variant}_call_sites"), f.call_site_count as f64);
        // Keep the entity-level opcode histogram modest: only record the
        // dominant opcode counts so a downstream consumer can answer
        // "what does this function spend itself on" without paying for
        // a sparse-everything-by-default record.
        for (op, count) in &f.opcode_histogram {
            metrics.insert(format!("{variant}_op_{op}"), *count as f64);
        }
        records.push(EntityRecord {
            // No FileId yet: LLVM IR symbols don't carry a source path
            // for the `define` line in a stable enough form for v1.
            // Stage 7's DWARF join supplies file/AstFuncId.
            ids: IdSet::default(),
            metrics: metrics
                .into_iter()
                .chain(std::iter::once(("name_length".into(), f.name.len() as f64)))
                .collect(),
        });
    }
    let key = format!("llvm.{}_func", variant);
    let mut map = BTreeMap::new();
    map.insert(key, records);
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_simple_function_into_one_record() {
        let text = "\
define i32 @add(i32 %a, i32 %b) {
start:
  %s = add i32 %a, %b
  ret i32 %s
}
";
        let funcs = parse_llvm_ir_text(text);
        assert_eq!(funcs.len(), 1);
        let f = &funcs[0];
        assert_eq!(f.name, "add");
        assert_eq!(f.instruction_count, 2, "add + ret");
        assert_eq!(f.basic_block_count, 1, "one labelled block — entry/start");
        assert_eq!(f.call_site_count, 0);
    }

    #[test]
    fn counts_basic_blocks_and_call_sites() {
        let text = "\
define i32 @classify(i32 %x) {
start:
  %t = icmp slt i32 %x, 0
  br i1 %t, label %neg, label %else
neg:
  %r1 = call i32 @abs(i32 %x)
  ret i32 %r1
else:
  ret i32 %x
}
";
        let funcs = parse_llvm_ir_text(text);
        assert_eq!(funcs.len(), 1);
        let f = &funcs[0];
        assert_eq!(f.basic_block_count, 3, "start + neg + else");
        assert_eq!(f.call_site_count, 1);
        assert!(f.instruction_count >= 5);
        assert!(f.opcode_histogram.get("call").copied() == Some(1));
        assert!(f.opcode_histogram.get("br").copied() == Some(1));
    }

    #[test]
    fn ignores_top_level_metadata_and_declarations() {
        let text = "\
; ModuleID = 'm'
target triple = \"x86_64-unknown-linux-gnu\"

declare void @ext(i32)

define void @main() {
  call void @ext(i32 0)
  ret void
}

!0 = !{i32 1, !\"x\", i32 2}
";
        let funcs = parse_llvm_ir_text(text);
        assert_eq!(funcs.len(), 1, "only @main is a definition");
        let f = &funcs[0];
        assert_eq!(f.name, "main");
        assert_eq!(f.call_site_count, 1);
    }

    #[test]
    fn tail_call_records_call_not_tail() {
        let f = parse_llvm_ir_text(
            "define void @f() {\n  tail call void @g()\n  ret void\n}\n",
        )
        .pop()
        .unwrap();
        assert_eq!(f.call_site_count, 1);
        assert!(f.opcode_histogram.get("call").copied() == Some(1));
        assert!(f.opcode_histogram.get("tail").copied().unwrap_or(0) == 0);
    }
}
