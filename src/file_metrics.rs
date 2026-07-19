use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast_parser;
use crate::flow::{self, EntryPointKind};
use crate::render;

pub struct FileFunction {
    pub graph_name: String,
    pub display_name: String,
    pub line_start: usize,
    pub line_end: usize,
    pub is_test: bool,
    pub is_bench: bool,
    pub is_test_support: bool,
    pub is_binary_wrapper_main: bool,
}

pub struct SourceSpanFilter {
    pub file: PathBuf,
    pub production_spans: Vec<(usize, usize)>,
    pub excluded_spans: Vec<(usize, usize)>,
}

pub struct ProductionFilter {
    pub files: Vec<SourceSpanFilter>,
}

impl ProductionFilter {
    pub fn includes_line(&self, file: &Path, line: usize) -> bool {
        self.files
            .iter()
            .find(|entry| same_path(&entry.file, file))
            .is_some_and(|entry| {
                entry
                    .production_spans
                    .iter()
                    .any(|(start, end)| line >= *start && line <= *end)
                    && !entry
                        .excluded_spans
                        .iter()
                        .any(|(start, end)| line >= *start && line <= *end)
            })
    }
}

pub struct FileFunctionIndex {
    pub functions: Vec<FileFunction>,
    #[allow(dead_code)]
    pub graph_name_to_index: HashMap<String, usize>,
}

pub fn selected_file_functions(
    symbols: &ast_parser::ProjectSymbols,
    graph: &flow::CallGraph,
    project_path: &Path,
    file: &Path,
) -> FileFunctionIndex {
    let file_module = render::module_name_from_path(file, project_path);
    let prefix = if file_module == "main" || file_module == "lib" {
        None
    } else {
        Some(file_module)
    };

    let mut functions: Vec<FileFunction> = symbols
        .files
        .iter()
        .filter(|symbols_file| symbols_file.path == file)
        .flat_map(|symbols_file| symbols_file.functions.iter())
        .map(|function| {
            let graph_name = match &prefix {
                Some(module) => format!("{}::{}", module, function.name),
                None => function.name.clone(),
            };
            let entry_kind = graph.entry_point_kinds.get(&graph_name);
            let node = graph.function_nodes.get(&graph_name);
            let is_test = matches!(entry_kind, Some(EntryPointKind::Test))
                || node.is_some_and(|node| node.is_test_entry);
            let is_bench = matches!(entry_kind, Some(EntryPointKind::Bench))
                || node.is_some_and(|node| node.is_bench_entry);
            let is_test_support = node
                .map(|node| node.is_test_support)
                .unwrap_or(function.is_test_support);
            let is_binary_wrapper_main = is_selected_bin_wrapper_main(file, &graph_name);

            FileFunction {
                display_name: graph_name.clone(),
                graph_name,
                line_start: function.line_start,
                line_end: function.line_end,
                is_test,
                is_bench,
                is_test_support,
                is_binary_wrapper_main,
            }
        })
        .collect();

    functions.sort_by(|a, b| {
        a.line_start
            .cmp(&b.line_start)
            .then(a.line_end.cmp(&b.line_end))
            .then(a.graph_name.cmp(&b.graph_name))
    });

    let graph_name_to_index = functions
        .iter()
        .enumerate()
        .map(|(index, function)| (function.graph_name.clone(), index))
        .collect();

    FileFunctionIndex {
        functions,
        graph_name_to_index,
    }
}

pub fn production_filter_for_files(
    symbols: &ast_parser::ProjectSymbols,
    graph: &flow::CallGraph,
    project_path: &Path,
    files: &[PathBuf],
    selected_file: Option<&Path>,
) -> ProductionFilter {
    let mut filters = Vec::new();
    for file in files {
        if selected_file.is_some_and(|selected| !same_path(selected, file)) {
            continue;
        }
        let index = selected_file_functions(symbols, graph, project_path, file);
        let mut production_spans = Vec::new();
        let mut excluded_spans = Vec::new();
        for function in &index.functions {
            let span = (function.line_start, function.line_end);
            if function.is_test
                || function.is_bench
                || function.is_test_support
                || function.is_binary_wrapper_main
            {
                excluded_spans.push(span);
            } else {
                production_spans.push(span);
            }
        }
        filters.push(SourceSpanFilter {
            file: file.clone(),
            production_spans,
            excluded_spans,
        });
    }
    ProductionFilter { files: filters }
}

fn is_selected_bin_wrapper_main(file: &Path, graph_name: &str) -> bool {
    (graph_name == "main" || graph_name.ends_with("::main")) && path_has_src_bin(file)
}

fn path_has_src_bin(path: &Path) -> bool {
    let mut saw_src = false;
    for component in path.components() {
        let name = component.as_os_str();
        if saw_src && name == "bin" {
            return true;
        }
        saw_src = name == "src";
    }
    false
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
