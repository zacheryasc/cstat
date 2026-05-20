//! Spec §2.6 levels 25–30: binary-artifact extractors and the AST↔Symbol
//! identity join (spec §6).
//!
//! These levels read the optimized build's `.rlib` and any raw `.o` left
//! in `target/cstat-opt/debug/deps/` through the `object` crate. For
//! library crates (the common case in v1) the rlib is an `ar` archive
//! holding one ELF object; we walk each member and aggregate. DWARF
//! debug info (level 29) supplies the linkage_name → source-name map
//! we use to attach `SymbolId`s onto `AstFuncId`s, giving the §13
//! worked example its joined `classify` record.
//!
//! All six levels gate on `Config.enable_codegen` because they consume
//! artifacts the Stage-6 codegen build produced. Without it, they
//! skip cleanly with a documented reason — no codegen vector keys
//! leak (spec §8).
//!
//! Level 30 (`link.map`) only emits when the build was driven with a
//! `-Wl,-Map=...` arg. For lib-only crates no link step runs, so this
//! level skips with reason naming the prerequisite — not a failure.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use cstat_orchestrate::{Context, CostClass, EntityRecord, Level, LevelError, LevelOutput};
use cstat_schema::{AstFuncId, FileId, IdSet, SymbolId};
use gimli::EndianSlice;
use object::read::archive::ArchiveFile;
use object::{Object, ObjectSection, ObjectSymbol, RelocationTarget, SectionIndex, SymbolKind};

use crate::ast_cache::SharedAst;
use crate::codegen_levels::SharedCodegen;

/// Lazy, shared parse of all object bytes produced by the codegen build.
///
/// The first level whose `extract` runs pays the parse cost; subsequent
/// levels reuse the cached aggregate. If the codegen build itself was
/// disabled or failed, every binary level translates that into the
/// same Skipped/Failed outcome so the user sees a consistent reason
/// across the whole §2.6 row.
#[derive(Clone)]
pub(crate) struct SharedBinary {
    codegen: SharedCodegen,
    ast: SharedAst,
    inner: Arc<OnceLock<BinaryResult>>,
}

type BinaryResult = Result<BinaryArtifacts, BinarySkip>;

#[derive(Debug, Clone)]
struct BinarySkip {
    reason: String,
    is_failure: bool,
}

impl BinarySkip {
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

/// Everything parsed out of the codegen build's object artifacts.
#[derive(Debug, Default)]
struct BinaryArtifacts {
    /// One entry per symbol across every object inside every rlib and
    /// every raw `.o`. Duplicate names across compilation units are
    /// preserved — section counts and size distributions want the
    /// raw view.
    symbols: Vec<SymbolRecord>,
    /// Per-section accumulator. Keyed by canonicalized section name
    /// (e.g. ".text" rather than each ".text.<symbol>" fragment).
    sections: BTreeMap<String, SectionAccum>,
    /// Number of relocation entries observed across all sections.
    relocation_count: u64,
    /// Count of relocations whose target is an external symbol (vs. a
    /// section-local relocation). A rough indirect-call density proxy.
    relocation_external_count: u64,
    /// One entry per DW_TAG_subprogram found in DWARF.
    subprograms: Vec<SubprogramRecord>,
    /// Number of inlined-subroutine DIEs.
    inlined_subroutine_count: u64,
    /// Number of compilation-unit DIEs (= compilation units in DWARF).
    compilation_unit_count: u64,
    /// How many distinct object files we successfully parsed.
    parsed_object_count: u64,
    /// How many archive members were found but couldn't be parsed as
    /// object files (e.g. legal-info members in some rlibs). Not an
    /// error — surfaced as a vector key so consumers can tell.
    skipped_member_count: u64,
}

#[derive(Debug)]
struct SymbolRecord {
    name: String,
    size: u64,
    kind: SymbolKindClass,
    is_global: bool,
    is_undefined: bool,
}

#[derive(Debug, Clone, Copy)]
enum SymbolKindClass {
    Function,
    Data,
    Section,
    Other,
}

#[derive(Debug, Default)]
struct SectionAccum {
    total_size: u64,
    fragment_count: u64,
}

#[derive(Debug, Clone)]
struct SubprogramRecord {
    /// DW_AT_name — short source-level name (e.g. "classify").
    short_name: Option<String>,
    /// DW_AT_linkage_name — the mangled symbol; matches a `SymbolId`.
    linkage_name: Option<String>,
}

impl SharedBinary {
    pub(crate) fn new(codegen: SharedCodegen, ast: SharedAst) -> Self {
        Self {
            codegen,
            ast,
            inner: Arc::new(OnceLock::new()),
        }
    }

    fn ast(&self) -> &SharedAst {
        &self.ast
    }

    fn get(&self, ctx: &Context) -> Result<&BinaryArtifacts, LevelError> {
        let result = self.inner.get_or_init(|| build_binary_artifacts(&self.codegen, ctx));
        match result {
            Ok(a) => Ok(a),
            Err(skip) => Err(skip.clone().into_level_error()),
        }
    }
}

fn build_binary_artifacts(codegen: &SharedCodegen, ctx: &Context) -> BinaryResult {
    // Inherit the codegen cache's outcome. If codegen is off / failed,
    // we surface the same diagnostic on the binary levels — the user
    // gets one consistent reason rather than five different ones.
    let cg = match codegen.get(ctx) {
        Ok(a) => a,
        Err(LevelError::Skipped(reason)) => return Err(BinarySkip::skipped(reason)),
        Err(LevelError::Failed(reason)) => return Err(BinarySkip::failed(reason)),
    };

    let mut artifacts = BinaryArtifacts::default();

    // Raw .o files (bin/cdylib targets emit these into deps/).
    for path in cg.object_files() {
        match std::fs::read(path) {
            Ok(bytes) => parse_object_bytes(&bytes, path, &mut artifacts),
            Err(_) => artifacts.skipped_member_count += 1,
        }
    }
    // Each rlib is an `ar` archive; walk members and parse the ones
    // that look like object files.
    for rlib in cg.rlib_files() {
        let Ok(bytes) = std::fs::read(rlib) else {
            artifacts.skipped_member_count += 1;
            continue;
        };
        let Ok(archive) = ArchiveFile::parse(&bytes[..]) else {
            artifacts.skipped_member_count += 1;
            continue;
        };
        for member in archive.members().flatten() {
            let Ok(data) = member.data(&bytes[..]) else {
                artifacts.skipped_member_count += 1;
                continue;
            };
            // rustc rlibs include a `lib.rmeta` member (Rust metadata)
            // and a `.rcgu.o` per codegen unit. Only the .o is a real
            // object; rmeta is not parseable by `object`.
            if member.name().ends_with(b".rmeta")
                || member.name().ends_with(b".rmeta_extra")
            {
                continue;
            }
            parse_object_bytes(data, Path::new(""), &mut artifacts);
        }
    }

    // Linker map: only present if the build was passed -Wl,-Map=...
    // We don't drive that in v1 (the spec §2.6 row 30 wording is
    // about *parsing* the map when one exists). Look for a few well-
    // known names anyway so a downstream consumer who passes their
    // own rustflags can still get a parsed level.
    artifacts_link_map(ctx, &mut artifacts);

    Ok(artifacts)
}

fn artifacts_link_map(_ctx: &Context, _artifacts: &mut BinaryArtifacts) {
    // No-op for now; LinkMapLevel handles its own filesystem probe.
}

fn parse_object_bytes(bytes: &[u8], _origin: &Path, out: &mut BinaryArtifacts) {
    let file = match object::File::parse(bytes) {
        Ok(f) => f,
        Err(_) => {
            out.skipped_member_count += 1;
            return;
        }
    };
    out.parsed_object_count += 1;

    for sym in file.symbols() {
        let kind = match sym.kind() {
            SymbolKind::Text => SymbolKindClass::Function,
            SymbolKind::Data | SymbolKind::Tls => SymbolKindClass::Data,
            SymbolKind::Section => SymbolKindClass::Section,
            _ => SymbolKindClass::Other,
        };
        // Section and File symbols inflate the count without carrying
        // useful per-symbol metrics; the §2.6 row-25 contract is the
        // post-link-style symbol table, which is mostly Text/Data.
        if matches!(kind, SymbolKindClass::Section | SymbolKindClass::Other)
            && sym.name().is_err()
        {
            continue;
        }
        let name = sym.name().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        out.symbols.push(SymbolRecord {
            name,
            size: sym.size(),
            kind,
            is_global: sym.is_global(),
            is_undefined: sym.is_undefined(),
        });
    }

    for section in file.sections() {
        let Ok(name) = section.name() else { continue };
        let canonical = canonicalize_section_name(name);
        let entry = out.sections.entry(canonical).or_default();
        entry.total_size += section.size();
        entry.fragment_count += 1;

        for (_, reloc) in section.relocations() {
            out.relocation_count += 1;
            if matches!(reloc.target(), RelocationTarget::Symbol(_)) {
                out.relocation_external_count += 1;
            }
        }
    }

    parse_dwarf(&file, out);
}

/// Bucket "`.text.foo`", "`.text.bar`" into a single "`.text`" entry. Rustc
/// emits one section per symbol under -ffunction-sections / -fdata-sections
/// by default; the aggregate is what spec §2.6 row 27 cares about.
fn canonicalize_section_name(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    // Walk through known prefixes; map .text.* -> .text, .data.* -> .data,
    // .rodata.* -> .rodata, .bss.* -> .bss, .debug_* -> .debug_*, etc.
    for prefix in [".text", ".data", ".rodata", ".bss", ".tdata", ".tbss"] {
        if raw == prefix
            || raw.starts_with(&format!("{prefix}."))
            || raw.starts_with(&format!("{prefix}.."))
        {
            return prefix.to_string();
        }
    }
    raw.to_string()
}

fn parse_dwarf<'a>(file: &object::File<'a, &'a [u8]>, out: &mut BinaryArtifacts) {
    let endian = if file.is_little_endian() {
        gimli::RunTimeEndian::Little
    } else {
        gimli::RunTimeEndian::Big
    };
    // ELF object files store DW_FORM_strp / DW_FORM_sec_offset / etc.
    // as zero in the section bytes, with a relocation entry that
    // carries the real offset. `uncompressed_data()` doesn't apply
    // those relocations, so every string offset comes back as 0 and
    // resolves to the first string in `.debug_str` (the producer
    // banner). Apply debug-section relocations ourselves so the
    // strings DWARF says are at offset N actually land at offset N.
    let load_section = |id: gimli::SectionId| -> Result<Vec<u8>, ()> {
        Ok(load_relocated_dwarf_section(file, id.name()))
    };
    let Ok(dwarf_sections) = gimli::DwarfSections::load(load_section) else {
        return;
    };
    let dwarf = dwarf_sections.borrow(|section| EndianSlice::new(section.as_slice(), endian));

    let mut units = dwarf.units();
    while let Ok(Some(header)) = units.next() {
        let Ok(unit) = dwarf.unit(header) else { continue };
        out.compilation_unit_count += 1;
        let mut entries = unit.entries();
        while let Ok(Some((_, entry))) = entries.next_dfs() {
            match entry.tag() {
                gimli::DW_TAG_subprogram => {
                    let mut short_name: Option<String> = None;
                    let mut linkage_name: Option<String> = None;
                    let mut attrs = entry.attrs();
                    while let Ok(Some(attr)) = attrs.next() {
                        let value = match attr.name() {
                            gimli::DW_AT_name => &mut short_name,
                            gimli::DW_AT_linkage_name => &mut linkage_name,
                            _ => continue,
                        };
                        if let Ok(bytes) = dwarf.attr_string(&unit, attr.value()) {
                            *value = Some(String::from_utf8_lossy(bytes.slice()).into_owned());
                        }
                    }
                    out.subprograms.push(SubprogramRecord {
                        short_name,
                        linkage_name,
                    });
                }
                gimli::DW_TAG_inlined_subroutine => {
                    out.inlined_subroutine_count += 1;
                }
                _ => {}
            }
        }
    }
}

// Public surface for the codegen cache so we can keep its fields
// private. Each accessor returns a borrowed slice of paths.
impl crate::codegen_levels::CodegenArtifacts {
    pub(crate) fn object_files(&self) -> &[PathBuf] { self.object_files_view() }
    pub(crate) fn rlib_files(&self) -> &[PathBuf] { self.rlib_files_view() }
}

/// Load a DWARF section's bytes with relocations applied.
///
/// In an unlinked ELF object, references like `DW_FORM_strp` are stored
/// as zero in the section bytes; the real value lives in a relocation
/// entry. Without this fix-up every string offset resolves to 0 and we
/// read the producer banner for every `DW_AT_name`. The patch is to
/// walk the section's relocations, compute the resolved value
/// (`target_address + addend`), and write it into the byte buffer at
/// the relocation's offset.
fn load_relocated_dwarf_section<'a>(file: &object::File<'a, &'a [u8]>, name: &str) -> Vec<u8> {
    let Some(section) = file.section_by_name(name) else {
        return Vec::new();
    };
    let Ok(cow) = section.uncompressed_data() else {
        return Vec::new();
    };
    let mut data = cow.into_owned();
    for (offset, rel) in section.relocations() {
        let off = offset as usize;
        let size = (rel.size() / 8) as usize;
        if size == 0 || size > 8 || off + size > data.len() {
            continue;
        }
        let target_value: u64 = match rel.target() {
            RelocationTarget::Symbol(idx) => file
                .symbol_by_index(idx)
                .map(|s| s.address())
                .unwrap_or(0),
            RelocationTarget::Section(idx) => section_address(file, idx),
            _ => 0,
        };
        let value = target_value.wrapping_add(rel.addend() as u64);
        let bytes = value.to_le_bytes();
        data[off..off + size].copy_from_slice(&bytes[..size]);
    }
    data
}

fn section_address<'a>(file: &object::File<'a, &'a [u8]>, idx: SectionIndex) -> u64 {
    for section in file.sections() {
        if section.index() == idx {
            return section.address();
        }
    }
    0
}

// ───── Level 25: symbol table ────────────────────────────────────────────

pub(crate) struct SymTableLevel {
    pub shared: SharedBinary,
}

impl Level for SymTableLevel {
    fn id(&self) -> &'static str { "sym.table" }
    fn cost_class(&self) -> CostClass { CostClass::C }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let bin = self.shared.get(ctx)?;
        if bin.symbols.is_empty() {
            return Err(LevelError::Failed(
                "no symbols found in any object file (rustc emitted nothing?)".into(),
            ));
        }
        let total = bin.symbols.len() as u64;
        let function_count = bin
            .symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKindClass::Function))
            .count() as u64;
        let data_count = bin
            .symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKindClass::Data))
            .count() as u64;
        let undefined_count = bin.symbols.iter().filter(|s| s.is_undefined).count() as u64;
        let global_count = bin.symbols.iter().filter(|s| s.is_global).count() as u64;

        let sizes: Vec<u64> = bin
            .symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKindClass::Function))
            .map(|s| s.size)
            .collect();
        let (size_max, size_mean, size_sum) = size_stats(&sizes);

        let vector = vec![
            ("sym.global.symbol_count".into(), total as f64),
            ("sym.global.function_count".into(), function_count as f64),
            ("sym.global.data_count".into(), data_count as f64),
            ("sym.global.undefined_count".into(), undefined_count as f64),
            ("sym.global.global_count".into(), global_count as f64),
            ("sym.global.parsed_object_count".into(), bin.parsed_object_count as f64),
            ("sym.global.skipped_member_count".into(), bin.skipped_member_count as f64),
            ("sym.func.size_bytes.sum".into(), size_sum),
            ("sym.func.size_bytes.mean".into(), size_mean),
            ("sym.func.size_bytes.max".into(), size_max),
        ];

        let mut records = Vec::new();
        for s in &bin.symbols {
            if !matches!(s.kind, SymbolKindClass::Function) {
                continue;
            }
            let mut metrics = BTreeMap::new();
            metrics.insert("size_bytes".into(), s.size as f64);
            metrics.insert("is_undefined".into(), bool_f(s.is_undefined));
            metrics.insert("is_global".into(), bool_f(s.is_global));
            metrics.insert("kind_is_function".into(), 1.0);
            records.push(EntityRecord {
                ids: IdSet {
                    symbol: Some(SymbolId::new(s.name.clone())),
                    ..Default::default()
                },
                metrics,
            });
        }
        let mut entities = BTreeMap::new();
        if !records.is_empty() {
            entities.insert("sym.func".into(), records);
        }

        Ok(LevelOutput { vector, entities })
    }
}

// ───── Level 26: demangled symbol graph ──────────────────────────────────

pub(crate) struct SymDemangledLevel {
    pub shared: SharedBinary,
}

impl Level for SymDemangledLevel {
    fn id(&self) -> &'static str { "sym.demangled" }
    fn cost_class(&self) -> CostClass { CostClass::L }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let bin = self.shared.get(ctx)?;
        let func_symbols: Vec<&SymbolRecord> = bin
            .symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKindClass::Function) && !s.is_undefined)
            .collect();
        if func_symbols.is_empty() {
            return Err(LevelError::Failed(
                "no defined function symbols available to demangle".into(),
            ));
        }
        let total = func_symbols.len() as u64;
        let mut demangled_ok = 0u64;
        let mut generic_instantiations = 0u64;
        let mut unique_sources: BTreeSet<String> = BTreeSet::new();
        for s in &func_symbols {
            let demangled = rustc_demangle::try_demangle(&s.name);
            match demangled {
                Ok(d) => {
                    demangled_ok += 1;
                    let pretty = format!("{d:#}");
                    if pretty.contains('<') && pretty.contains('>') {
                        generic_instantiations += 1;
                    }
                    // Strip the trailing `::h<hash>` rustc adds to every
                    // mangled symbol to recover the "source-level" name.
                    let source = strip_rustc_hash(&pretty);
                    unique_sources.insert(source);
                }
                Err(_) => {
                    // C-style or unmangled symbol; still counts as a
                    // distinct source-level name.
                    unique_sources.insert(s.name.clone());
                }
            }
        }
        // spec §13: source-to-symbol ratio is mono-pressure: how many
        // mangled symbols exist per distinct source-level function?
        let source_to_symbol_ratio = if !unique_sources.is_empty() {
            total as f64 / unique_sources.len() as f64
        } else {
            0.0
        };

        let vector = vec![
            ("sym.demangled.function_count".into(), total as f64),
            ("sym.demangled.demangled_ok_count".into(), demangled_ok as f64),
            (
                "sym.demangled.generic_instantiation_count".into(),
                generic_instantiations as f64,
            ),
            (
                "sym.demangled.unique_source_function_count".into(),
                unique_sources.len() as f64,
            ),
            (
                "sym.global.source_to_symbol_ratio".into(),
                source_to_symbol_ratio,
            ),
        ];

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

fn strip_rustc_hash(pretty: &str) -> String {
    // rustc adds `::h<16 hex chars>` as the disambiguator. The
    // demangler's non-`#` mode already strips it, but the `#`-mode
    // (legacy) keeps it; we apply both paths defensively.
    if let Some(idx) = pretty.rfind("::h") {
        let tail = &pretty[idx + 3..];
        if tail.len() == 16 && tail.chars().all(|c| c.is_ascii_hexdigit()) {
            return pretty[..idx].to_string();
        }
    }
    pretty.to_string()
}

// ───── Level 27: section layout ──────────────────────────────────────────

pub(crate) struct SymSectionsLevel {
    pub shared: SharedBinary,
}

impl Level for SymSectionsLevel {
    fn id(&self) -> &'static str { "sym.sections" }
    fn cost_class(&self) -> CostClass { CostClass::C }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let bin = self.shared.get(ctx)?;
        if bin.sections.is_empty() {
            return Err(LevelError::Failed(
                "no sections found in any object file".into(),
            ));
        }
        let total_bytes: u64 = bin.sections.values().map(|a| a.total_size).sum();
        let total_fragments: u64 = bin.sections.values().map(|a| a.fragment_count).sum();
        let text = bin.sections.get(".text").map(|a| a.total_size).unwrap_or(0);
        let data = bin.sections.get(".data").map(|a| a.total_size).unwrap_or(0);
        let rodata = bin.sections.get(".rodata").map(|a| a.total_size).unwrap_or(0);
        let bss = bin.sections.get(".bss").map(|a| a.total_size).unwrap_or(0);
        let debug_total: u64 = bin
            .sections
            .iter()
            .filter(|(name, _)| name.starts_with(".debug"))
            .map(|(_, a)| a.total_size)
            .sum();

        let vector = vec![
            ("sym.sections.total_bytes".into(), total_bytes as f64),
            ("sym.sections.section_count".into(), bin.sections.len() as f64),
            ("sym.sections.fragment_count".into(), total_fragments as f64),
            ("sym.sections.text_bytes".into(), text as f64),
            ("sym.sections.data_bytes".into(), data as f64),
            ("sym.sections.rodata_bytes".into(), rodata as f64),
            ("sym.sections.bss_bytes".into(), bss as f64),
            ("sym.sections.debug_bytes".into(), debug_total as f64),
            (
                "sym.sections.text_ratio".into(),
                ratio(text, total_bytes),
            ),
            (
                "sym.sections.debug_ratio".into(),
                ratio(debug_total, total_bytes),
            ),
        ];

        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Level 28: relocations ─────────────────────────────────────────────

pub(crate) struct SymRelocsLevel {
    pub shared: SharedBinary,
}

impl Level for SymRelocsLevel {
    fn id(&self) -> &'static str { "sym.relocs" }
    fn cost_class(&self) -> CostClass { CostClass::C }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let bin = self.shared.get(ctx)?;
        // No relocations is not a failure on a tiny lib crate (the
        // optimizer may emit none for trivial bodies). Surface the
        // zero rather than failing the level.
        let total = bin.relocation_count;
        let external = bin.relocation_external_count;
        let internal = total.saturating_sub(external);
        let vector = vec![
            ("sym.relocs.total".into(), total as f64),
            ("sym.relocs.external_target_count".into(), external as f64),
            ("sym.relocs.internal_target_count".into(), internal as f64),
            (
                "sym.relocs.indirect_call_density_proxy".into(),
                ratio(external, total.max(1)),
            ),
        ];
        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── Level 29: DWARF debug info (and AST↔Symbol join) ──────────────────

pub(crate) struct DwarfLevel {
    pub shared: SharedBinary,
}

impl Level for DwarfLevel {
    fn id(&self) -> &'static str { "dwarf" }
    fn cost_class(&self) -> CostClass { CostClass::C }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        let bin = self.shared.get(ctx)?;
        if bin.subprograms.is_empty() {
            return Err(LevelError::Failed(
                "no DW_TAG_subprogram entries in object DWARF (was the build stripped?)".into(),
            ));
        }

        let total = bin.subprograms.len() as u64;
        let with_linkage = bin
            .subprograms
            .iter()
            .filter(|s| s.linkage_name.is_some())
            .count() as u64;
        let inline_ratio = if total > 0 {
            bin.inlined_subroutine_count as f64 / total as f64
        } else {
            0.0
        };

        // Build the join. Spec §6: every per-entity record carries the
        // set of IDs by which it is known. For each AST function whose
        // short name matches exactly one DWARF subprogram with a
        // linkage name, emit a record under `entities.ast.func` with
        // both AstFuncId and SymbolId. Ambiguous (zero or >1 match)
        // cases get AstFuncId only.
        let ast = self.shared.ast().get(&ctx.project_root);

        // Index DWARF subprograms by their short name. Multi-counted
        // matches lose the join — we can't pick deterministically.
        let mut name_to_linkages: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for sp in &bin.subprograms {
            let Some(short) = sp.short_name.as_deref() else { continue };
            let Some(linkage) = sp.linkage_name.as_deref() else { continue };
            name_to_linkages.entry(short).or_default().push(linkage);
        }

        let mut join_attempts = 0u64;
        let mut join_successes = 0u64;
        let mut records = Vec::new();
        for f in ast.iter_functions() {
            join_attempts += 1;
            let candidates = name_to_linkages.get(f.name.as_str()).cloned().unwrap_or_default();
            let symbol = if candidates.len() == 1 {
                join_successes += 1;
                Some(SymbolId::new(candidates[0].to_string()))
            } else {
                None
            };
            let ast_func_id = AstFuncId {
                file: FileId::new(posix(&f.file_path)),
                fq_path: f.fq_path.clone(),
                item_kind: "fn".into(),
                ast_node_hash: f.node_hash.clone(),
            };
            let mut metrics = BTreeMap::new();
            metrics.insert("symbol_join_ok".into(), bool_f(symbol.is_some()));
            records.push(EntityRecord {
                ids: IdSet {
                    file: Some(FileId::new(posix(&f.file_path))),
                    ast_func: Some(ast_func_id),
                    symbol,
                    ..Default::default()
                },
                metrics,
            });
        }
        let join_failure_rate = if join_attempts > 0 {
            (join_attempts - join_successes) as f64 / join_attempts as f64
        } else {
            0.0
        };

        let vector = vec![
            ("dwarf.global.subprogram_count".into(), total as f64),
            (
                "dwarf.global.subprograms_with_linkage_count".into(),
                with_linkage as f64,
            ),
            (
                "dwarf.global.compilation_unit_count".into(),
                bin.compilation_unit_count as f64,
            ),
            (
                "dwarf.global.inlined_subroutine_count".into(),
                bin.inlined_subroutine_count as f64,
            ),
            ("dwarf.global.inline_ratio".into(), inline_ratio),
            (
                "dwarf.global.join_attempt_count".into(),
                join_attempts as f64,
            ),
            (
                "dwarf.global.join_success_count".into(),
                join_successes as f64,
            ),
            ("dwarf.global.join_failure_rate".into(), join_failure_rate),
        ];

        let mut entities = BTreeMap::new();
        if !records.is_empty() {
            entities.insert("ast.func".into(), records);
        }

        Ok(LevelOutput { vector, entities })
    }
}

// ───── Level 30: linker dependency graph ────────────────────────────────

pub(crate) struct LinkMapLevel {
    pub shared: SharedBinary,
}

impl Level for LinkMapLevel {
    fn id(&self) -> &'static str { "link.map" }
    fn cost_class(&self) -> CostClass { CostClass::C }
    fn deterministic(&self) -> bool { true }

    fn extract(&self, ctx: &Context) -> Result<LevelOutput, LevelError> {
        // Cause the codegen cache to run so this level shares the same
        // disabled/failed message as its siblings when codegen is off.
        let _ = self.shared.get(ctx)?;

        // The linker map exists only when the build was driven with
        // -Wl,-Map=<path>. v1 doesn't surface a flag for that, but we
        // still probe the well-known locations so a downstream user
        // who passes their own RUSTFLAGS gets a parsed level.
        let candidates = [
            ctx.project_root.join("target").join("cstat-opt").join("debug").join("link.map"),
            ctx.project_root.join("target").join("link.map"),
        ];
        let Some(path) = candidates.iter().find(|p| p.is_file()) else {
            return Err(LevelError::Skipped(
                "no linker map file (build did not pass -Wl,-Map=...; lib-only crates skip link)".into(),
            ));
        };
        let text = std::fs::read_to_string(path)
            .map_err(|e| LevelError::Failed(format!("read {path:?}: {e}")))?;
        let line_count = text.lines().count() as f64;
        let object_mentions = text
            .lines()
            .filter(|l| l.contains(".o") || l.contains(".rlib"))
            .count() as f64;
        let vector = vec![
            ("link.map.line_count".into(), line_count),
            ("link.map.object_mention_count".into(), object_mentions),
        ];
        Ok(LevelOutput { vector, ..Default::default() })
    }
}

// ───── helpers ──────────────────────────────────────────────────────────

fn size_stats(sizes: &[u64]) -> (f64, f64, f64) {
    if sizes.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let sum: u64 = sizes.iter().sum();
    let max = *sizes.iter().max().unwrap();
    let mean = sum as f64 / sizes.len() as f64;
    (max as f64, mean, sum as f64)
}

fn ratio(num: u64, denom: u64) -> f64 {
    if denom == 0 { 0.0 } else { num as f64 / denom as f64 }
}

fn bool_f(b: bool) -> f64 { if b { 1.0 } else { 0.0 } }

fn posix(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_name_canonicalization_buckets_per_symbol_sections() {
        assert_eq!(canonicalize_section_name(".text"), ".text");
        assert_eq!(canonicalize_section_name(".text.classify"), ".text");
        assert_eq!(canonicalize_section_name(".data"), ".data");
        assert_eq!(canonicalize_section_name(".data.foo"), ".data");
        assert_eq!(canonicalize_section_name(".rodata.str.0"), ".rodata");
        assert_eq!(canonicalize_section_name(".bss.x"), ".bss");
        // Debug sections stay distinct.
        assert_eq!(canonicalize_section_name(".debug_info"), ".debug_info");
        assert_eq!(canonicalize_section_name(".note.gnu.property"), ".note.gnu.property");
    }

    #[test]
    fn strip_rustc_hash_removes_the_disambiguator_suffix() {
        assert_eq!(
            strip_rustc_hash("single_file_lib::classify::h0123456789abcdef"),
            "single_file_lib::classify"
        );
        // Wrong length: leave intact.
        assert_eq!(
            strip_rustc_hash("foo::bar::h01"),
            "foo::bar::h01"
        );
        // No ::h suffix: leave intact.
        assert_eq!(strip_rustc_hash("foo::bar"), "foo::bar");
    }

    #[test]
    fn ratio_is_zero_on_empty_denominator() {
        assert_eq!(ratio(0, 0), 0.0);
        assert_eq!(ratio(7, 0), 0.0);
        assert_eq!(ratio(3, 6), 0.5);
    }
}
