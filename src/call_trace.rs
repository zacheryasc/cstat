use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::ast_parser;
use crate::file_metrics;
use crate::flow::{self, CallGraph};
use crate::render;

#[derive(Debug, Clone, Serialize)]
struct ResolvedEntry {
    source: EntrySource,
    package: Option<String>,
    target: Option<String>,
    entry_file: Option<String>,
    requested: Option<String>,
    function: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EntrySource {
    CargoRun,
    Explicit,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TraceNode {
    pub function: String,
    pub cycle: bool,
    pub children: Vec<TraceNode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionTraceSummary {
    pub function: String,
    pub functions_reached: usize,
    pub max_depth: usize,
    pub direct_callees: Vec<String>,
    pub tree: TraceNode,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileCallTraceReport {
    pub file: String,
    pub functions: Vec<FunctionTraceSummary>,
}

#[derive(Debug, Clone, Serialize)]
struct CallTraceJson {
    cstat_version: String,
    resolved: ResolvedEntry,
    tree: TraceNode,
    functions_reached: usize,
    max_depth: usize,
}

#[derive(Debug, Clone, Serialize)]
struct EntryFilesJson {
    cstat_version: String,
    kind: &'static str,
    entry: String,
    directory: String,
    files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct EntryFunctionsJson {
    cstat_version: String,
    kind: &'static str,
    entry: String,
    file: String,
    functions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum EntryListing {
    Files(EntryFilesJson),
    Functions(EntryFunctionsJson),
}

#[derive(Debug, Default)]
struct ManifestInfo {
    package: Option<String>,
    default_run: Option<String>,
    bins: Vec<ManifestBin>,
}

#[derive(Debug, Default)]
struct ManifestBin {
    name: Option<String>,
    path: Option<String>,
}

#[derive(Debug, Clone)]
struct BinCandidate {
    name: String,
    path: PathBuf,
    relative_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestSection {
    None,
    Package,
    Bin,
}

/// Render the call trace command.
pub fn run(
    rs_files: &[PathBuf],
    project_path: &Path,
    entry: Option<&str>,
    json: bool,
    verbose: bool,
) -> Result<(), String> {
    if let Some(requested) = entry {
        if let Some(listing) = maybe_entry_listing(project_path, requested)? {
            if json {
                println!("{}", serde_json::to_string(&listing).unwrap());
            } else {
                render_entry_listing(&listing);
            }
            return Ok(());
        }
    }

    let output = analyze(rs_files, project_path, entry)?;

    if json {
        println!("{}", serde_json::to_string(&output).unwrap());
    } else {
        render_human(&output, verbose);
    }

    Ok(())
}

fn analyze(
    rs_files: &[PathBuf],
    project_path: &Path,
    entry: Option<&str>,
) -> Result<CallTraceJson, String> {
    let graph = flow::build_call_graph(rs_files, project_path);
    if graph.functions.is_empty() {
        return Err("no functions found to analyze".to_string());
    }

    let resolved = match entry {
        Some(requested) => resolve_explicit_entry(project_path, &graph, requested)?,
        None => resolve_cargo_run_entry(project_path, &graph)?,
    };

    let (tree, functions_reached, max_depth) = build_trace_tree(&graph, &resolved.function);

    Ok(CallTraceJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        resolved,
        tree,
        functions_reached,
        max_depth,
    })
}

pub fn run_file(
    rs_files: &[PathBuf],
    project_path: &Path,
    file: &Path,
    entry: Option<&str>,
    json: bool,
    verbose: bool,
) -> Result<(), String> {
    let symbols = ast_parser::parse_project(rs_files);
    let graph = flow::build_call_graph(rs_files, project_path);
    let file_functions =
        file_metrics::selected_file_functions(&symbols, &graph, project_path, file);
    let file_display_path = relative_path_string(project_path, file);
    let report = analyze_file_call_traces(&graph, &file_functions, &file_display_path);

    let output = if let Some(entry) = entry {
        let function = resolve_file_entry(&report, entry)?;
        FileCallTraceReport {
            file: report.file.clone(),
            functions: vec![function.clone()],
        }
    } else {
        report
    };

    if json {
        println!("{}", serde_json::to_string(&output).unwrap());
    } else {
        render_file_call_trace_report(&output, verbose);
    }

    Ok(())
}

pub fn analyze_file_call_traces(
    graph: &CallGraph,
    file_functions: &file_metrics::FileFunctionIndex,
    file_display_path: &str,
) -> FileCallTraceReport {
    let mut functions: Vec<FunctionTraceSummary> = file_functions
        .functions
        .iter()
        .filter(|function| !function.is_test && !function.is_bench)
        .map(|function| {
            let direct_callees = graph
                .adjacency
                .get(&function.graph_name)
                .cloned()
                .unwrap_or_default();
            let (tree, functions_reached, max_depth) =
                if graph.functions.contains(&function.graph_name) {
                    build_trace_tree(graph, &function.graph_name)
                } else {
                    (
                        TraceNode {
                            function: function.graph_name.clone(),
                            cycle: false,
                            children: Vec::new(),
                        },
                        1,
                        1,
                    )
                };

            FunctionTraceSummary {
                function: function.display_name.clone(),
                functions_reached,
                max_depth,
                direct_callees,
                tree,
            }
        })
        .collect();

    functions.sort_by(|a, b| {
        b.functions_reached
            .cmp(&a.functions_reached)
            .then(b.max_depth.cmp(&a.max_depth))
            .then(a.function.cmp(&b.function))
    });

    FileCallTraceReport {
        file: file_display_path.to_string(),
        functions,
    }
}

fn resolve_file_entry(
    report: &FileCallTraceReport,
    requested: &str,
) -> Result<FunctionTraceSummary, String> {
    if let Some(exact) = report
        .functions
        .iter()
        .find(|function| function.function == requested)
    {
        return Ok(exact.clone());
    }

    let suffix = format!("::{requested}");
    let matches: Vec<&FunctionTraceSummary> = report
        .functions
        .iter()
        .filter(|function| function.function.ends_with(&suffix) || function.function == requested)
        .collect();

    if matches.len() == 1 {
        Ok(matches[0].clone())
    } else {
        Err(format!(
            "entry function not found or ambiguous: {requested}"
        ))
    }
}
fn maybe_entry_listing(
    project_path: &Path,
    requested: &str,
) -> Result<Option<EntryListing>, String> {
    if requested.contains("::") {
        return Ok(None);
    }

    let Some(path) = resolve_existing_entry_path(project_path, requested)? else {
        return Ok(None);
    };

    if path.is_dir() {
        return Ok(Some(EntryListing::Files(list_parseable_entry_files(
            project_path,
            requested,
            &path,
        ))));
    }

    if path.is_file() && path.extension().is_some_and(|extension| extension == "rs") {
        return Ok(Some(EntryListing::Functions(list_entry_functions(
            project_path,
            requested,
            &path,
        )?)));
    }

    Err(format!(
        "entry path is neither a Rust file nor a directory: {requested}"
    ))
}

fn resolve_existing_entry_path(
    project_path: &Path,
    requested: &str,
) -> Result<Option<PathBuf>, String> {
    let raw = Path::new(requested);
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        project_path.join(raw)
    };

    if !candidate.exists() {
        return Ok(None);
    }

    let path = candidate
        .canonicalize()
        .map_err(|error| format!("cannot resolve entry path {requested}: {error}"))?;

    if !path.starts_with(project_path) {
        return Err(format!("entry path is outside project: {requested}"));
    }

    Ok(Some(path))
}

fn list_parseable_entry_files(
    project_path: &Path,
    requested: &str,
    directory: &Path,
) -> EntryFilesJson {
    let mut files: Vec<PathBuf> = WalkDir::new(directory)
        .into_iter()
        .filter_map(Result::ok)
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.is_file() && path.extension().is_some_and(|extension| extension == "rs")
        })
        .collect();
    files.sort();

    let symbols = ast_parser::parse_project(&files);
    let parseable_files = symbols
        .files
        .into_iter()
        .filter(|file| !file.parse_error)
        .map(|file| relative_path_string(project_path, &file.path))
        .collect();

    EntryFilesJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        kind: "files",
        entry: requested.to_string(),
        directory: relative_path_string(project_path, directory),
        files: parseable_files,
    }
}

fn list_entry_functions(
    project_path: &Path,
    requested: &str,
    file: &Path,
) -> Result<EntryFunctionsJson, String> {
    let symbols = ast_parser::parse_project(&[file.to_path_buf()]);
    let Some(file_symbols) = symbols.files.into_iter().next() else {
        return Err(format!("entry file could not be parsed: {requested}"));
    };
    if file_symbols.parse_error {
        return Err(format!("entry file could not be parsed: {requested}"));
    }

    let file_path = relative_path_string(project_path, file);
    let functions = file_symbols
        .functions
        .into_iter()
        .map(|function| format!("{file_path}::{}", function.name))
        .collect();

    Ok(EntryFunctionsJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        kind: "functions",
        entry: requested.to_string(),
        file: file_path,
        functions,
    })
}

fn resolve_cargo_run_entry(
    project_path: &Path,
    graph: &CallGraph,
) -> Result<ResolvedEntry, String> {
    let manifest = read_manifest(project_path)?;
    let candidates = binary_candidates(project_path, &manifest);
    let selected = select_candidate(&manifest, &candidates)?;

    if !selected.path.exists() {
        return Err(format!(
            "cargo run target source not found: {}",
            selected.relative_path
        ));
    }

    let function = main_function_name(project_path, &selected.path);
    if !graph.functions.contains(&function) {
        return Err(format!(
            "entry function not found: {}::main",
            selected.relative_path
        ));
    }

    Ok(ResolvedEntry {
        source: EntrySource::CargoRun,
        package: manifest.package,
        target: Some(selected.name),
        entry_file: Some(selected.relative_path),
        requested: None,
        function,
    })
}

fn resolve_explicit_entry(
    project_path: &Path,
    graph: &CallGraph,
    requested: &str,
) -> Result<ResolvedEntry, String> {
    let package = package_name(project_path);

    let (function, entry_file) = if let Some(index) = requested.find(".rs::") {
        let file_part = &requested[..index + 3];
        let function_part = &requested[index + 5..];
        if function_part.is_empty() {
            return Err(format!("entry function not found: {requested}"));
        }

        let path = PathBuf::from(file_part);
        let full_path = if path.is_absolute() {
            path
        } else {
            project_path.join(path)
        };
        let full_path = full_path
            .canonicalize()
            .map_err(|_| format!("entry file not found: {file_part}"))?;
        if !full_path.starts_with(project_path) {
            return Err(format!("entry path is outside project: {file_part}"));
        }

        let module = render::module_name_from_path(&full_path, project_path);
        let graph_name = if module == "main" || module == "lib" {
            function_part.to_string()
        } else {
            format!("{module}::{function_part}")
        };
        (
            graph_name,
            Some(relative_path_string(project_path, &full_path)),
        )
    } else {
        if !graph.functions.iter().any(|function| function == requested) {
            return Err(format!("entry function not found: {requested}"));
        }
        (requested.to_string(), None)
    };

    if !graph.functions.contains(&function) {
        return Err(format!("entry function not found: {requested}"));
    }

    Ok(ResolvedEntry {
        source: EntrySource::Explicit,
        package,
        target: None,
        entry_file,
        requested: Some(requested.to_string()),
        function,
    })
}

fn read_manifest(project_path: &Path) -> Result<ManifestInfo, String> {
    let manifest_path = project_path.join("Cargo.toml");
    let content = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("cannot read Cargo.toml: {error}"))?;
    Ok(parse_manifest(&content))
}

fn package_name(project_path: &Path) -> Option<String> {
    read_manifest(project_path)
        .ok()
        .and_then(|manifest| manifest.package)
}

fn parse_manifest(content: &str) -> ManifestInfo {
    let mut manifest = ManifestInfo::default();
    let mut section = ManifestSection::None;
    let mut current_bin: Option<ManifestBin> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        match trimmed {
            "[package]" => {
                finish_bin(&mut manifest, &mut current_bin);
                section = ManifestSection::Package;
                continue;
            }
            "[[bin]]" => {
                finish_bin(&mut manifest, &mut current_bin);
                current_bin = Some(ManifestBin::default());
                section = ManifestSection::Bin;
                continue;
            }
            _ if trimmed.starts_with('[') => {
                finish_bin(&mut manifest, &mut current_bin);
                section = ManifestSection::None;
                continue;
            }
            _ => {}
        }

        let Some((key, value)) = parse_string_assignment(trimmed) else {
            continue;
        };

        match section {
            ManifestSection::Package => match key {
                "name" => manifest.package = Some(value),
                "default-run" => manifest.default_run = Some(value),
                _ => {}
            },
            ManifestSection::Bin => {
                if let Some(bin) = current_bin.as_mut() {
                    match key {
                        "name" => bin.name = Some(value),
                        "path" => bin.path = Some(value),
                        _ => {}
                    }
                }
            }
            ManifestSection::None => {}
        }
    }

    finish_bin(&mut manifest, &mut current_bin);
    manifest
}

fn finish_bin(manifest: &mut ManifestInfo, current_bin: &mut Option<ManifestBin>) {
    if let Some(bin) = current_bin.take() {
        manifest.bins.push(bin);
    }
}

fn parse_string_assignment(line: &str) -> Option<(&str, String)> {
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    let value = value.trim_start();
    let value = value.strip_prefix('"')?;
    let end = value.find('"')?;
    Some((key, value[..end].to_string()))
}

fn binary_candidates(project_path: &Path, manifest: &ManifestInfo) -> Vec<BinCandidate> {
    let mut candidates = Vec::new();

    for bin in &manifest.bins {
        let Some(name) = bin.name.clone() else {
            continue;
        };
        let path = bin
            .path
            .as_ref()
            .map(|path| project_path.join(path))
            .unwrap_or_else(|| {
                project_path
                    .join("src")
                    .join("bin")
                    .join(format!("{name}.rs"))
            });
        push_candidate(&mut candidates, candidate(project_path, name, path));
    }

    let main_path = project_path.join("src").join("main.rs");
    if main_path.exists() {
        let name = manifest
            .package
            .clone()
            .unwrap_or_else(|| "main".to_string());
        push_candidate(&mut candidates, candidate(project_path, name, main_path));
    }

    let bin_dir = project_path.join("src").join("bin");
    if let Ok(entries) = fs::read_dir(bin_dir) {
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
            .collect();
        paths.sort();

        for path in paths {
            let Some(name) = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
            else {
                continue;
            };
            push_candidate(&mut candidates, candidate(project_path, name, path));
        }
    }

    candidates
}

fn candidate(project_path: &Path, name: String, path: PathBuf) -> BinCandidate {
    let relative_path = relative_path_string(project_path, &path);
    BinCandidate {
        name,
        path,
        relative_path,
    }
}

fn push_candidate(candidates: &mut Vec<BinCandidate>, candidate: BinCandidate) {
    if candidates
        .iter()
        .any(|existing| existing.name == candidate.name && existing.path == candidate.path)
    {
        return;
    }
    candidates.push(candidate);
}

fn select_candidate(
    manifest: &ManifestInfo,
    candidates: &[BinCandidate],
) -> Result<BinCandidate, String> {
    if let Some(default_run) = manifest.default_run.as_deref() {
        return candidates
            .iter()
            .find(|candidate| candidate.name == default_run)
            .cloned()
            .ok_or_else(|| format!("default-run target not found: {default_run}"));
    }

    if candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }

    Err("cannot infer cargo run target; pass --entry PATH::FUNCTION".to_string())
}

fn main_function_name(project_path: &Path, path: &Path) -> String {
    let module = render::module_name_from_path(path, project_path);
    if module == "main" || module == "lib" {
        "main".to_string()
    } else {
        format!("{module}::main")
    }
}

pub(crate) fn relative_path_string(project_path: &Path, path: &Path) -> String {
    path.strip_prefix(project_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(crate) fn build_trace_tree(graph: &CallGraph, entry: &str) -> (TraceNode, usize, usize) {
    let mut stack = HashSet::new();
    let mut reached = HashSet::new();
    stack.insert(entry.to_string());
    reached.insert(entry.to_string());

    let tree = build_trace_node(graph, entry, &mut stack, &mut reached);
    let max_depth = expanded_depth(&tree);
    (tree, reached.len(), max_depth)
}

fn build_trace_node(
    graph: &CallGraph,
    function: &str,
    stack: &mut HashSet<String>,
    reached: &mut HashSet<String>,
) -> TraceNode {
    let mut children = Vec::new();

    if let Some(callees) = graph.adjacency.get(function) {
        for callee in callees {
            if stack.contains(callee) {
                children.push(TraceNode {
                    function: callee.clone(),
                    cycle: true,
                    children: Vec::new(),
                });
                continue;
            }

            stack.insert(callee.clone());
            reached.insert(callee.clone());
            children.push(build_trace_node(graph, callee, stack, reached));
            stack.remove(callee);
        }
    }

    TraceNode {
        function: function.to_string(),
        cycle: false,
        children,
    }
}

fn expanded_depth(node: &TraceNode) -> usize {
    if node.cycle {
        return 0;
    }

    1 + node.children.iter().map(expanded_depth).max().unwrap_or(0)
}

fn render_entry_listing(listing: &EntryListing) {
    match listing {
        EntryListing::Files(output) => {
            println!("parseable files under {}:", output.directory);
            if output.files.is_empty() {
                println!("  <none>");
            } else {
                for file in &output.files {
                    println!("  {file}");
                }
            }
        }
        EntryListing::Functions(output) => {
            println!("entrypoints in {}:", output.file);
            if output.functions.is_empty() {
                println!("  <none>");
            } else {
                for function in &output.functions {
                    println!("  {function}");
                }
            }
        }
    }
}

fn render_human(output: &CallTraceJson, verbose: bool) {
    println!("cstat call trace");
    if verbose {
        render::verbose_block(&[
            "Static call graph rendered from one selected root.",
            "Cycles stop expansion; shared functions expand in each stack.",
        ]);
    }

    println!("resolved:");
    match output.resolved.source {
        EntrySource::CargoRun => {
            println!("  cargo command: cargo run --");
            println!(
                "  package: {}",
                output.resolved.package.as_deref().unwrap_or("unknown")
            );
            println!(
                "  target: bin {}",
                output.resolved.target.as_deref().unwrap_or("unknown")
            );
            println!(
                "  entry: {}::main",
                output.resolved.entry_file.as_deref().unwrap_or("unknown")
            );
        }
        EntrySource::Explicit => {
            if let Some(package) = output.resolved.package.as_deref() {
                println!("  package: {package}");
            }
            println!(
                "  entry: {}",
                output
                    .resolved
                    .requested
                    .as_deref()
                    .unwrap_or(output.resolved.function.as_str())
            );
        }
    }

    println!();
    render_trace_node(&output.tree);
    println!();
    println!("summary:");
    println!("  functions reached: {}", output.functions_reached);
    println!("  max depth: {}", output.max_depth);
}

fn render_file_call_trace_report(report: &FileCallTraceReport, verbose: bool) {
    println!("cstat file call trace ({})", report.file);
    if verbose {
        render::verbose_block(&[
            "Static call graph rendered for every non-test function in the selected file.",
            "Cycles stop expansion; shared functions expand in each stack.",
        ]);
    }

    if report.functions.is_empty() {
        println!("  <none>");
        return;
    }

    for function in &report.functions {
        println!();
        println!(
            "{} (functions reached: {}, max depth: {}, direct callees: {})",
            function.function,
            function.functions_reached,
            function.max_depth,
            if function.direct_callees.is_empty() {
                "-".to_string()
            } else {
                function.direct_callees.join(", ")
            }
        );
        render_trace_node(&function.tree);
    }
}

fn render_trace_node(node: &TraceNode) {
    println!("{}", format_node(node));
    render_children(&node.children, "");
}

fn render_children(children: &[TraceNode], prefix: &str) {
    let count = children.len();
    for (index, child) in children.iter().enumerate() {
        let is_last = index + 1 == count;
        let has_children = !child.children.is_empty();
        let connector = if is_last { "└── " } else { "├── " };
        let connector = if has_children {
            connector.replace("── ", "─┬ ")
        } else {
            connector.to_string()
        };
        println!("{prefix}{connector}{}", format_node(child));

        let child_prefix = if is_last { "    " } else { "│   " };
        let next_prefix = format!("{prefix}{child_prefix}");
        render_children(&child.children, &next_prefix);
    }
}

fn format_node(node: &TraceNode) -> String {
    if node.cycle {
        format!("{} ⟲ cycle", node.function)
    } else {
        node.function.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_project(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("cstat-call-trace-{name}-{unique}"));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        root
    }

    #[test]
    fn resolves_conventional_cargo_run_entry() {
        let root = temp_project("default-entry");
        let main = root.join("src/main.rs");
        fs::write(&main, "fn main() { parse(); }\nfn parse() {}\n").unwrap();

        let graph = flow::build_call_graph(&[main], &root);
        let resolved = resolve_cargo_run_entry(&root, &graph).unwrap();

        assert_eq!(resolved.source, EntrySource::CargoRun);
        assert_eq!(resolved.package.as_deref(), Some("fixture"));
        assert_eq!(resolved.target.as_deref(), Some("fixture"));
        assert_eq!(resolved.entry_file.as_deref(), Some("src/main.rs"));
        assert_eq!(resolved.function, "main");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_explicit_file_entry() {
        let root = temp_project("explicit-entry");
        let main = root.join("src/main.rs");
        fs::write(&main, "fn main() { parse(); }\nfn parse() {}\n").unwrap();

        let graph = flow::build_call_graph(&[main], &root);
        let resolved = resolve_explicit_entry(&root, &graph, "src/main.rs::parse").unwrap();

        assert_eq!(resolved.source, EntrySource::Explicit);
        assert_eq!(resolved.requested.as_deref(), Some("src/main.rs::parse"));
        assert_eq!(resolved.entry_file.as_deref(), Some("src/main.rs"));
        assert_eq!(resolved.function, "parse");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn builds_trace_tree_from_root() {
        let root = temp_project("tree");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            "fn main() { parse(); }\nfn parse() { leaf(); }\nfn leaf() {}\n",
        )
        .unwrap();

        let graph = flow::build_call_graph(&[main], &root);
        let (tree, functions_reached, max_depth) = build_trace_tree(&graph, "main");

        assert_eq!(tree.function, "main");
        assert_eq!(tree.children.len(), 1);
        assert_eq!(tree.children[0].function, "parse");
        assert_eq!(tree.children[0].children.len(), 1);
        assert_eq!(tree.children[0].children[0].function, "leaf");
        assert_eq!(functions_reached, 3);
        assert_eq!(max_depth, 3);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn expands_shared_function_in_different_stacks() {
        let root = temp_project("shared");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            "fn main() { left(); right(); }\nfn left() { shared(); }\nfn right() { shared(); }\nfn shared() {}\n",
        )
        .unwrap();

        let graph = flow::build_call_graph(&[main], &root);
        let (tree, functions_reached, max_depth) = build_trace_tree(&graph, "main");

        assert_eq!(tree.children.len(), 2);
        assert_eq!(tree.children[0].function, "left");
        assert_eq!(tree.children[0].children[0].function, "shared");
        assert!(!tree.children[0].children[0].cycle);
        assert_eq!(tree.children[1].function, "right");
        assert_eq!(tree.children[1].children[0].function, "shared");
        assert!(!tree.children[1].children[0].cycle);
        assert_eq!(functions_reached, 4);
        assert_eq!(max_depth, 3);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn marks_deep_stack_cycle() {
        let root = temp_project("deep-cycle");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            "fn main() { a(); }\nfn a() { b(); }\nfn b() { c(); }\nfn c() { d(); }\nfn d() { e(); }\nfn e() { f(); }\nfn f() { g(); }\nfn g() { b(); }\n",
        )
        .unwrap();

        let graph = flow::build_call_graph(&[main], &root);
        let (tree, functions_reached, max_depth) = build_trace_tree(&graph, "main");

        let mut node = &tree;
        for expected in ["main", "a", "b", "c", "d", "e", "f", "g"] {
            assert_eq!(node.function, expected);
            if expected != "g" {
                assert_eq!(node.children.len(), 1);
                node = &node.children[0];
            }
        }
        assert_eq!(node.children.len(), 1);
        let cycle = &node.children[0];
        assert_eq!(cycle.function, "b");
        assert!(cycle.cycle);
        assert!(cycle.children.is_empty());
        assert_eq!(functions_reached, 8);
        assert_eq!(max_depth, 8);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn builds_unbounded_deep_trace() {
        let root = temp_project("deep");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            "fn main() { f1(); }\nfn f1() { f2(); }\nfn f2() { f3(); }\nfn f3() { f4(); }\nfn f4() { f5(); }\nfn f5() { f6(); }\nfn f6() { f7(); }\nfn f7() { f8(); }\nfn f8() { f9(); }\nfn f9() { f10(); }\nfn f10() { f11(); }\nfn f11() { f12(); }\nfn f12() { f13(); }\nfn f13() { f14(); }\nfn f14() {}\n",
        )
        .unwrap();

        let graph = flow::build_call_graph(std::slice::from_ref(&main), &root);
        let (tree, functions_reached, max_depth) = build_trace_tree(&graph, "main");

        let mut node = &tree;
        for expected in [
            "main", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12",
            "f13", "f14",
        ] {
            assert_eq!(node.function, expected);
            if expected != "f14" {
                assert_eq!(node.children.len(), 1);
                node = &node.children[0];
            }
        }
        assert!(node.children.is_empty());
        assert_eq!(functions_reached, 15);
        assert_eq!(max_depth, 15);
        let output = analyze(
            std::slice::from_ref(&main),
            &root,
            Some("src/main.rs::main"),
        )
        .unwrap();
        let json = serde_json::to_string(&output).unwrap();
        assert!(!json.contains("depth_limited"));
        assert!(!json.contains("repeated"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lists_parseable_files_for_entry_directory() {
        let root = temp_project("entry-dir");
        let main = root.join("src/main.rs");
        let broken = root.join("src/broken.rs");
        fs::write(&main, "fn main() {}\n").unwrap();
        fs::write(&broken, "fn broken(\n").unwrap();

        let listing = maybe_entry_listing(&root, "src").unwrap().unwrap();

        let EntryListing::Files(output) = listing else {
            panic!("expected file listing");
        };
        assert_eq!(output.directory, "src");
        assert_eq!(output.files, vec!["src/main.rs".to_string()]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lists_entrypoints_for_entry_file() {
        let root = temp_project("entry-file");
        let main = root.join("src/main.rs");
        fs::write(
            &main,
            "fn main() { parse(); }\nfn parse() {}\nstruct Runner;\nimpl Runner { fn start(&self) {} }\n",
        )
        .unwrap();

        let listing = maybe_entry_listing(&root, "src/main.rs").unwrap().unwrap();

        let EntryListing::Functions(output) = listing else {
            panic!("expected function listing");
        };
        assert_eq!(output.file, "src/main.rs");
        assert_eq!(
            output.functions,
            vec![
                "src/main.rs::main".to_string(),
                "src/main.rs::parse".to_string(),
                "src/main.rs::Runner::start".to_string(),
            ]
        );

        fs::remove_dir_all(root).unwrap();
    }
}
