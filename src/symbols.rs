use colored::Colorize;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::ast_parser::{self, ProjectSymbols};
use crate::render;

#[derive(Debug, Clone, Serialize, Default)]
pub struct SymbolTotals {
    pub total: usize,
    pub functions: usize,
    pub structs: usize,
    pub enums: usize,
    pub traits: usize,
    pub trait_impls: usize,
    pub consts: usize,
    pub statics: usize,
    pub parse_error_files: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileSymbolCounts {
    pub path: String,
    pub total: usize,
    pub functions: usize,
    pub structs: usize,
    pub enums: usize,
    pub traits: usize,
    pub trait_impls: usize,
    pub consts: usize,
    pub statics: usize,
    pub parse_error: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolReport {
    pub totals: SymbolTotals,
    pub files: Vec<FileSymbolCounts>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileSymbolKind {
    Fn,
    Struct,
    Enum,
    Trait,
    TraitImpl,
    Const,
    Static,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileSymbolRow {
    pub kind: FileSymbolKind,
    pub symbol: String,
    pub line_start: Option<usize>,
    pub line_end: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileSymbolReport {
    pub file: String,
    pub total: usize,
    pub functions: usize,
    pub structs: usize,
    pub enums: usize,
    pub traits: usize,
    pub trait_impls: usize,
    pub consts: usize,
    pub statics: usize,
    pub parse_error: bool,
    pub symbols: Vec<FileSymbolRow>,
}

pub fn analyze_symbols(symbols: &ProjectSymbols, project_path: &Path) -> SymbolReport {
    let mut files: Vec<FileSymbolCounts> = symbols
        .files
        .iter()
        .map(|file| {
            let functions = file.functions.len();
            let structs = file.structs.len();
            let enums = file.enums.len();
            let traits = file.traits.len();
            let trait_impls = file.impls.iter().filter(|i| i.trait_name.is_some()).count();
            let consts = file.consts.len();
            let statics = file.statics.len();
            let total = functions + structs + enums + traits + trait_impls + consts + statics;

            FileSymbolCounts {
                path: strip_prefix(&file.path, project_path).display().to_string(),
                total,
                functions,
                structs,
                enums,
                traits,
                trait_impls,
                consts,
                statics,
                parse_error: file.parse_error,
            }
        })
        .collect();

    files.sort_by(|a, b| b.total.cmp(&a.total).then(a.path.cmp(&b.path)));

    let totals = files.iter().fold(SymbolTotals::default(), |mut acc, file| {
        acc.total += file.total;
        acc.functions += file.functions;
        acc.structs += file.structs;
        acc.enums += file.enums;
        acc.traits += file.traits;
        acc.trait_impls += file.trait_impls;
        acc.consts += file.consts;
        acc.statics += file.statics;
        if file.parse_error {
            acc.parse_error_files += 1;
        }
        acc
    });

    SymbolReport { totals, files }
}

pub fn analyze_files(rs_files: &[PathBuf], project_path: &Path) -> SymbolReport {
    let symbols = ast_parser::parse_project(rs_files);
    analyze_symbols(&symbols, project_path)
}

pub fn analyze_symbols_file(
    symbols: &ast_parser::ProjectSymbols,
    project_path: &Path,
    file: &Path,
) -> FileSymbolReport {
    let file_display = relative_path_string(project_path, file);
    let Some(file_symbols) = symbols.files.iter().find(|entry| entry.path == file) else {
        return FileSymbolReport {
            file: file_display,
            total: 0,
            functions: 0,
            structs: 0,
            enums: 0,
            traits: 0,
            trait_impls: 0,
            consts: 0,
            statics: 0,
            parse_error: false,
            symbols: Vec::new(),
        };
    };

    let mut rows = Vec::new();
    for function in &file_symbols.functions {
        rows.push(FileSymbolRow {
            kind: FileSymbolKind::Fn,
            symbol: function.name.clone(),
            line_start: Some(function.line_start),
            line_end: Some(function.line_end),
        });
    }
    for item in &file_symbols.structs {
        rows.push(FileSymbolRow {
            kind: FileSymbolKind::Struct,
            symbol: item.name.clone(),
            line_start: Some(item.line_start),
            line_end: Some(item.line_end),
        });
    }
    for item in &file_symbols.enums {
        rows.push(FileSymbolRow {
            kind: FileSymbolKind::Enum,
            symbol: item.name.clone(),
            line_start: Some(item.line_start),
            line_end: Some(item.line_end),
        });
    }
    for item in &file_symbols.traits {
        rows.push(FileSymbolRow {
            kind: FileSymbolKind::Trait,
            symbol: item.name.clone(),
            line_start: Some(item.line_start),
            line_end: Some(item.line_end),
        });
    }
    for item in &file_symbols.impls {
        if let Some(trait_name) = &item.trait_name {
            rows.push(FileSymbolRow {
                kind: FileSymbolKind::TraitImpl,
                symbol: format!("<{} as {}>", item.target_type, trait_name),
                line_start: Some(item.line_start),
                line_end: Some(item.line_end),
            });
        }
    }
    for item in &file_symbols.consts {
        rows.push(FileSymbolRow {
            kind: FileSymbolKind::Const,
            symbol: item.name.clone(),
            line_start: Some(item.line_start),
            line_end: Some(item.line_end),
        });
    }
    for item in &file_symbols.statics {
        rows.push(FileSymbolRow {
            kind: FileSymbolKind::Static,
            symbol: item.name.clone(),
            line_start: Some(item.line_start),
            line_end: Some(item.line_end),
        });
    }

    let functions = file_symbols.functions.len();
    let structs = file_symbols.structs.len();
    let enums = file_symbols.enums.len();
    let traits = file_symbols.traits.len();
    let trait_impls = file_symbols
        .impls
        .iter()
        .filter(|item| item.trait_name.is_some())
        .count();
    let consts = file_symbols.consts.len();
    let statics = file_symbols.statics.len();
    let total = functions + structs + enums + traits + trait_impls + consts + statics;

    FileSymbolReport {
        file: file_display,
        total,
        functions,
        structs,
        enums,
        traits,
        trait_impls,
        consts,
        statics,
        parse_error: file_symbols.parse_error,
        symbols: rows,
    }
}

pub fn render_symbol_report(report: &SymbolReport, verbose: bool) {
    render::section_header("Symbols");

    if verbose {
        render::verbose_block(&[
            "symbols uses syn Rust AST item discovery.",
            "It does not perform semantic name resolution, rustc analysis, macro expansion, or public API usage proof.",
            "Functions include free functions and methods in any impl block; methods are counted once as functions.",
            "trait_impls counts only impl Trait for Type blocks; inherent impl Type containers are not counted separately.",
            "Parse-error files are surfaced with zero counts and parse_error=true in JSON.",
        ]);
    }

    println!(
        "  {} {}  {} {}  {} {}  {} {}  {} {}  {} {}  {} {}  {} {}",
        "total".cyan(),
        report.totals.total.to_string().bold(),
        "fn".cyan(),
        report.totals.functions.to_string().bold(),
        "struct".cyan(),
        report.totals.structs.to_string().bold(),
        "enum".cyan(),
        report.totals.enums.to_string().bold(),
        "trait".cyan(),
        report.totals.traits.to_string().bold(),
        "trait_impl".cyan(),
        report.totals.trait_impls.to_string().bold(),
        "const".cyan(),
        report.totals.consts.to_string().bold(),
        "static".cyan(),
        report.totals.statics.to_string().bold(),
    );

    if report.totals.parse_error_files > 0 {
        println!(
            "  {} {}",
            "parse errors:".yellow().bold(),
            report.totals.parse_error_files
        );
    }

    println!();
    println!(
        "  {:<48} {:>5} {:>5} {:>6} {:>5} {:>6} {:>11} {:>6} {:>6}",
        "file".bold(),
        "total".bold(),
        "fn".bold(),
        "struct".bold(),
        "enum".bold(),
        "trait".bold(),
        "trait_impl".bold(),
        "const".bold(),
        "static".bold()
    );
    for file in &report.files {
        let marker = if file.parse_error { " !" } else { "" };
        println!(
            "  {:<48} {:>5} {:>5} {:>6} {:>5} {:>6} {:>11} {:>6} {:>6}",
            truncate(&format!("{}{}", file.path, marker), 48),
            file.total,
            file.functions,
            file.structs,
            file.enums,
            file.traits,
            file.trait_impls,
            file.consts,
            file.statics,
        );
    }
}

pub fn render_symbols(rs_files: &[PathBuf], project_path: &Path, verbose: bool) {
    let report = analyze_files(rs_files, project_path);
    render_symbol_report(&report, verbose);
}

pub fn render_symbols_json(rs_files: &[PathBuf], project_path: &Path) {
    let report = analyze_files(rs_files, project_path);
    println!("{}", serde_json::to_string(&report).unwrap());
}

pub fn render_symbols_file(rs_files: &[PathBuf], project_path: &Path, file: &Path, verbose: bool) {
    let symbols = ast_parser::parse_project(rs_files);
    let report = analyze_symbols_file(&symbols, project_path, file);
    render_file_symbol_report(&report, verbose);
}

pub fn render_symbols_file_json(rs_files: &[PathBuf], project_path: &Path, file: &Path) {
    let symbols = ast_parser::parse_project(rs_files);
    let report = analyze_symbols_file(&symbols, project_path, file);
    println!("{}", serde_json::to_string(&report).unwrap());
}

pub fn render_file_symbol_report(report: &FileSymbolReport, verbose: bool) {
    render::section_header("Symbols");

    if verbose {
        render::verbose_block(&[
            "Selected-file mode is used when --path points at a Rust source file.",
            "symbols uses syn Rust AST item discovery; it does not perform semantic name resolution, rustc analysis, macro expansion, or public API usage proof.",
            "Functions include free functions and methods in any impl block; methods are counted once as functions.",
            "trait_impls counts only impl Trait for Type blocks; inherent impl Type containers are not counted separately.",
            "Line spans come from syn/proc-macro2 token spans for Rust item rows.",
        ]);
    }

    println!(
        "  {} {}  {} {}  {} {}  {} {}  {} {}  {} {}  {} {}  {} {}",
        "total".cyan(),
        report.total.to_string().bold(),
        "fn".cyan(),
        report.functions.to_string().bold(),
        "struct".cyan(),
        report.structs.to_string().bold(),
        "enum".cyan(),
        report.enums.to_string().bold(),
        "trait".cyan(),
        report.traits.to_string().bold(),
        "trait_impl".cyan(),
        report.trait_impls.to_string().bold(),
        "const".cyan(),
        report.consts.to_string().bold(),
        "static".cyan(),
        report.statics.to_string().bold(),
    );

    if report.parse_error {
        println!(
            "  {}",
            "parse error: selected file could not be parsed"
                .yellow()
                .bold()
        );
    }

    println!();
    println!(
        "  {:<10} {:<48} {}",
        "kind".bold(),
        "symbol".bold(),
        "lines".bold()
    );
    for row in &report.symbols {
        let lines = match (row.line_start, row.line_end) {
            (Some(start), Some(end)) => format!("{start}-{end}"),
            _ => "-".to_string(),
        };
        println!(
            "  {:<10} {:<48} {}",
            file_symbol_kind_label(&row.kind),
            truncate(&row.symbol, 48),
            lines,
        );
    }
}

fn file_symbol_kind_label(kind: &FileSymbolKind) -> &'static str {
    match kind {
        FileSymbolKind::Fn => "fn",
        FileSymbolKind::Struct => "struct",
        FileSymbolKind::Enum => "enum",
        FileSymbolKind::Trait => "trait",
        FileSymbolKind::TraitImpl => "trait_impl",
        FileSymbolKind::Const => "const",
        FileSymbolKind::Static => "static",
    }
}

fn relative_path_string(project_path: &Path, path: &Path) -> String {
    path.strip_prefix(project_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn strip_prefix<'a>(path: &'a Path, base: &Path) -> &'a Path {
    path.strip_prefix(base).unwrap_or(path)
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
