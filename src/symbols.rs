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
    pub impls: usize,
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
    pub impls: usize,
    pub parse_error: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolReport {
    pub totals: SymbolTotals,
    pub files: Vec<FileSymbolCounts>,
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
            let impls = file.impls.len();
            let total = functions + structs + enums + traits + impls;

            FileSymbolCounts {
                path: strip_prefix(&file.path, project_path).display().to_string(),
                total,
                functions,
                structs,
                enums,
                traits,
                impls,
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
        acc.impls += file.impls;
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

pub fn render_symbol_report(report: &SymbolReport, verbose: bool) {
    render::section_header("Symbols");

    if verbose {
        render::verbose_block(&[
            "Counts are Rust AST items found in parseable files.",
            "Functions includes free functions and impl methods.",
            "Parse-error files are listed with zero symbols so the report stays deterministic.",
        ]);
    }

    println!(
        "  {} {}  {} {}  {} {}  {} {}  {} {}  {} {}",
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
        "impl".cyan(),
        report.totals.impls.to_string().bold(),
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
        "  {:<48} {:>5} {:>5} {:>6} {:>5} {:>6} {:>5}",
        "file".bold(),
        "total".bold(),
        "fn".bold(),
        "struct".bold(),
        "enum".bold(),
        "trait".bold(),
        "impl".bold()
    );
    for file in &report.files {
        let marker = if file.parse_error { " !" } else { "" };
        println!(
            "  {:<48} {:>5} {:>5} {:>6} {:>5} {:>6} {:>5}",
            truncate(&format!("{}{}", file.path, marker), 48),
            file.total,
            file.functions,
            file.structs,
            file.enums,
            file.traits,
            file.impls,
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
