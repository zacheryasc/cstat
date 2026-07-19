mod sections;

use colored::Colorize;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::ast_parser;
use crate::circuits;
use crate::complexity;
use crate::datapaths;
use crate::deps;
use crate::graph;
use crate::loc;
use crate::render;

use sections::*;

/// Render the summary dashboard — the default view combining all analyses.
pub fn render_summary(rs_files: &[PathBuf], project_path: &Path, verbose: bool) {
    let term_w = render::terminal_width();
    let project_path_canon = project_path
        .canonicalize()
        .unwrap_or_else(|_| project_path.to_path_buf());

    // ── Gather data ──────────────────────────────────────────
    let (file_stats, agg_loc) = loc::analyze_files(rs_files);
    let symbols = ast_parser::parse_project(rs_files);
    let func_complexities = complexity::compute_all(&symbols, &project_path_canon);
    let complexity_stats = complexity::compute_stats(&func_complexities);
    let dep_analysis = deps::analyze_deps(rs_files, &project_path_canon);

    // ── Project name ─────────────────────────────────────────
    let project_name = project_path_canon
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let total_files = file_stats.len();
    let total_loc = agg_loc.as_ref().map_or(0, |a| a.total_loc);
    let total_functions = func_complexities.len();
    let total_modules = dep_analysis.modules.len();

    // ── Render each section ──────────────────────────────────
    render_summary_header(
        &project_name,
        total_files,
        total_loc,
        total_functions,
        total_modules,
        term_w,
        verbose,
    );
    render_loc_section(&file_stats, &project_path_canon, term_w, total_loc, verbose);
    render_complexity_section(&func_complexities, verbose);
    render_connectome_section(&dep_analysis, term_w, verbose);
    render_distributions_section(&func_complexities, verbose);
    render_centrality_section(rs_files, &project_path_canon, verbose);
    render_reachability_section(rs_files, &project_path_canon, verbose);
    render_circuits_section(rs_files, &project_path_canon, verbose);
    render_stats_footer(
        &agg_loc,
        &complexity_stats,
        total_files,
        total_functions,
        total_modules,
        verbose,
    );
}

/// Render the dashboard header banner.
fn render_summary_header(
    project_name: &str,
    total_files: usize,
    total_loc: usize,
    total_functions: usize,
    total_modules: usize,
    term_w: usize,
    verbose: bool,
) {
    let header_line = "═".repeat(term_w.min(60));
    println!("\n{}", header_line.bright_cyan().bold());
    println!(
        "{}  {}",
        "cstat".bright_cyan().bold(),
        project_name.white().bold()
    );
    println!(
        "{} files  ·  {} code lines  ·  {} functions  ·  {} modules",
        format!("{}", total_files).bold(),
        format!("{}", total_loc).bold(),
        format!("{}", total_functions).bold(),
        format!("{}", total_modules).bold(),
    );
    println!("{}", header_line.bright_cyan().bold());

    if verbose {
        render::verbose_block(&[
            "This dashboard gives a bird's-eye view of your codebase.",
            "Each section below summarizes a different dimension of the project:",
            "size, complexity, dependencies, graph structure, execution flow, and modularity.",
            "",
            "Tip: For machine-readable output, use 'cstat dump' to get a structured JSON",
            "diagnostic report, or pass --json to any subcommand for raw structured data.",
            "Run 'cstat guide' for a topic-based reference explaining what each metric means.",
        ]);
    }
}

/// Render the Lines of Code section with top-10 chart and directory breakdown.
fn render_loc_section(
    file_stats: &[loc::FileLocStats],
    project_path_canon: &Path,
    term_w: usize,
    total_loc: usize,
    verbose: bool,
) {
    render::section_header("Lines of Code — Top Files");

    if verbose {
        render::verbose_block(&[
            "Files ranked by non-blank, non-comment source lines of code (LoC).",
            "Bar length is proportional to LoC. Color gradient: red = largest, green = smallest.",
            "Only the top 10 files are shown; the full list is available via 'cstat loc'.",
        ]);
    }

    if file_stats.is_empty() {
        println!("  {}", "No source files found.".dimmed());
    } else {
        render_loc_bars(file_stats, project_path_canon, term_w);

        if verbose {
            println!();
            render::verbose_block(&[
                "Directory breakdown: proportional LoC by top-level directory.",
                "Bar size = share of total code. Color: red > 50%, yellow > 25%, green otherwise.",
            ]);
        }
        render_dir_breakdown(file_stats, project_path_canon, term_w, total_loc);
    }
}

/// Render the LoC bar chart for up to 10 files.
fn render_loc_bars(file_stats: &[loc::FileLocStats], project_path_canon: &Path, term_w: usize) {
    let display_count = file_stats.len().min(10);
    let display_stats = &file_stats[..display_count];
    let max_label_len = display_stats
        .iter()
        .map(|f| short_path(&f.path, project_path_canon).len())
        .max()
        .unwrap_or(0)
        .min(35);
    let max_loc = display_stats
        .iter()
        .map(|f| f.code_lines)
        .max()
        .unwrap_or(1);
    let loc_digits = format!("{}", max_loc).len();
    let bar_budget = term_w
        .saturating_sub(max_label_len + 2)
        .saturating_sub(3)
        .saturating_sub(loc_digits)
        .saturating_sub(2)
        .max(10);

    for (i, stat) in display_stats.iter().enumerate() {
        let ratio = i as f64 / display_count.max(1) as f64;
        let label = short_path_truncated(&stat.path, project_path_canon, max_label_len);
        let bar_len = if max_loc > 0 {
            (stat.code_lines as f64 / max_loc as f64 * bar_budget as f64).ceil() as usize
        } else {
            0
        }
        .max(if stat.code_lines > 0 { 1 } else { 0 });

        let bar = render::bar_color(&"█".repeat(bar_len), ratio);
        let padding = " ".repeat(bar_budget.saturating_sub(bar_len));

        println!(
            "  {:>width$} │ {}{} {}",
            label.dimmed(),
            bar,
            padding,
            format!("{}", stat.code_lines).bold(),
            width = max_label_len,
        );
    }
    if file_stats.len() > display_count {
        println!(
            "  {}",
            format!("... and {} more files", file_stats.len() - display_count).dimmed()
        );
    }
}

/// Render the complexity hotspots table.
fn render_complexity_section(func_complexities: &[complexity::FunctionComplexity], verbose: bool) {
    render::section_header("Complexity Hotspots");

    if verbose {
        render::verbose_block(&[
            "Functions ranked by composite complexity score (highest first).",
            "Composite Score = 1.0×Cyclomatic + 0.5×Cognitive + 0.3×Nesting + 0.2×Generics + 0.1×Params.",
            "Cycl = cyclomatic complexity = branch points + 1 (linearly independent paths).",
            "Cogn = cognitive complexity (SonarSource model): +1 per control flow break, +nesting penalty.",
            "Nest = deepest nesting depth within the function body.",
            "Score column: red = highest complexity, green = lowest. See 'cstat complexity' for full detail.",
        ]);
    }

    if func_complexities.is_empty() {
        println!("  {}", "No functions found.".dimmed());
    } else {
        let display_count = func_complexities.len().min(10);
        println!(
            "  {:<36} {:>5} {:>5} {:>5} {:>7}",
            "Function".bold().underline(),
            "Cycl".bold().underline(),
            "Cogn".bold().underline(),
            "Nest".bold().underline(),
            "Score".bold().underline(),
        );

        for (i, fc) in func_complexities.iter().take(display_count).enumerate() {
            let ratio = i as f64 / display_count.max(1) as f64;
            let name = if fc.name.len() > 34 {
                format!("{}...", &fc.name[..31])
            } else {
                fc.name.clone()
            };
            let score_str = format!("{:.1}", fc.composite_score);
            let colored_score = render::bar_color(&score_str, ratio);

            println!(
                "  {:<36} {:>5} {:>5} {:>5} {}",
                name.dimmed(),
                fc.cyclomatic,
                fc.cognitive,
                fc.nesting_depth,
                colored_score,
            );
        }
        if func_complexities.len() > display_count {
            println!(
                "  {}",
                format!(
                    "... and {} more functions",
                    func_complexities.len() - display_count
                )
                .dimmed()
            );
        }
    }
}

/// Render the module connectome heatmap and coupling statistics.
fn render_connectome_section(dep_analysis: &deps::DepAnalysis, term_w: usize, verbose: bool) {
    render::section_header("Module Connectome");

    if verbose {
        render::verbose_block(&[
            "Heatmap of inter-module dependencies built from 'use' and 'mod' statements.",
            "Cell values: 0 (dim) = no dependency, 1 (yellow) = unidirectional, 2 (red) = bidirectional.",
            "Diagonal = · (a module cannot depend on itself).",
            "Bidirectional (2, red) means both modules import each other — a coupling risk.",
            "Top coupled pairs: ◆ bidi (red) = bidirectional, → uni (yellow) = one-way.",
            "Fan-in (blue) = how many modules depend on me (high = core utility).",
            "Fan-out (yellow) = how many modules I depend on (high = potential god module).",
        ]);
    }

    if dep_analysis.modules.is_empty() || dep_analysis.edges.is_empty() {
        println!("  {}", "No inter-module dependencies detected.".dimmed());
    } else {
        render_mini_heatmap(dep_analysis, term_w);
        render_connectome_details(dep_analysis);
    }
}

/// Render coupling pairs, fan-in/fan-out, and edge counts for the connectome.
fn render_connectome_details(dep_analysis: &deps::DepAnalysis) {
    let coupling_display = dep_analysis.coupling_pairs.len().min(5);
    if coupling_display > 0 {
        println!("  {}", "Top coupled pairs:".bold());
        for (a, b, strength) in dep_analysis.coupling_pairs.iter().take(coupling_display) {
            let kind = if *strength >= 2 {
                "◆ bidi".red().bold().to_string()
            } else {
                "→ uni".yellow().to_string()
            };
            println!("    {} ↔ {}  {}", a.dimmed(), b.dimmed(), kind);
        }
    }

    println!();
    let mut by_fan_in: Vec<(&String, usize)> = dep_analysis
        .in_degree
        .iter()
        .map(|(m, d)| (m, *d))
        .filter(|(_, d)| *d > 0)
        .collect();
    by_fan_in.sort_by(|a, b| b.1.cmp(&a.1));

    let mut by_fan_out: Vec<(&String, usize)> = dep_analysis
        .out_degree
        .iter()
        .map(|(m, d)| (m, *d))
        .filter(|(_, d)| *d > 0)
        .collect();
    by_fan_out.sort_by(|a, b| b.1.cmp(&a.1));

    if let Some((name, deg)) = by_fan_in.first() {
        println!(
            "  {} {} (fan-in: {})",
            "Highest fan-in:".bold(),
            name.bright_blue(),
            format!("{}", deg).bold(),
        );
    }
    if let Some((name, deg)) = by_fan_out.first() {
        println!(
            "  {} {} (fan-out: {})",
            "Highest fan-out:".bold(),
            name.yellow(),
            format!("{}", deg).bold(),
        );
    }

    let total_edges = dep_analysis.edges.len();
    let bidi_count = dep_analysis
        .coupling_pairs
        .iter()
        .filter(|(_, _, s)| *s >= 2)
        .count();
    println!(
        "  {} edges, {} bidirectional pairs",
        format!("{}", total_edges).bold(),
        format!("{}", bidi_count).bold(),
    );
}

/// Render the sparkline metric distributions section.
fn render_distributions_section(
    func_complexities: &[complexity::FunctionComplexity],
    verbose: bool,
) {
    render::section_header("Metric Distributions");

    if verbose {
        render::verbose_block(&[
            "Sparklines show the sorted distribution shape for each metric.",
            "Values are sorted ascending, then binned into ~40 buckets to form the sparkline.",
            "▁ = low, █ = high. A flat line means uniform values; a steep ramp means outliers.",
            "Range shows min–max. μ = arithmetic mean across all functions.",
        ]);
    }

    if !func_complexities.is_empty() {
        render_sparkline_distributions(func_complexities);
    } else {
        println!("  {}", "No functions to analyze.".dimmed());
    }
}

/// Render the graph centrality highlights section.
fn render_centrality_section(rs_files: &[PathBuf], project_path_canon: &Path, verbose: bool) {
    render::section_header("Graph Centrality");

    if verbose {
        render::verbose_block(&[
            "Key nodes in the call graph, identified by three complementary metrics.",
            "bc = betweenness centrality (Brandes algorithm): fraction of all shortest paths",
            "  passing through this node. High bc = bridge/bottleneck — removing it would",
            "  disconnect parts of the graph. Scale: 0.0 (no shortest paths) to 1.0 (all paths).",
            "pr = PageRank (damping 0.85, 100 iterations): recursive importance — a node is",
            "  important if important nodes point to it. Higher = more central.",
            "cc = clustering coefficient: fraction of a node's neighbors that are also connected",
            "  to each other. High cc = tight clique; low cc = broker between groups.",
        ]);
    }

    match graph::graph_summary(rs_files, project_path_canon) {
        Some(gs) => render_graph_centrality(&gs),
        None => println!("  {}", "No call graph data available.".dimmed()),
    }
}

/// Render the static entrypoint reachability section.
fn render_reachability_section(rs_files: &[PathBuf], project_path_canon: &Path, verbose: bool) {
    render::section_header("Static entrypoint reachability");

    if verbose {
        render::verbose_block(&[
            "Percentage of functions reachable from entry points via static call graph traversal.",
            "Entry points are main(), #[test] functions, and benchmarks. Reachability is computed via BFS.",
            "Reachability = reachable / total. Cold functions = unreachable = dead code candidates.",
            "Color: green > 80%, yellow > 50%, red ≤ 50%.",
        ]);
    }

    match datapaths::entrypoint_reachability_summary(rs_files, project_path_canon) {
        Some(ds) => render_entrypoint_reachability(&ds),
        None => println!("  {}", "No reachability data available.".dimmed()),
    }
}

/// Render the circuits overview section.
fn render_circuits_section(rs_files: &[PathBuf], project_path_canon: &Path, verbose: bool) {
    render::section_header("Circuits");

    if verbose {
        render::verbose_block(&[
            "Communities detected by label propagation on the call graph.",
            "Each node starts with its own label, then iteratively adopts the most frequent",
            "neighbor label until stable. The result is functional clusters (circuits).",
            "Modularity Q measures partition quality: Q > 0.3 (green) = good modular structure,",
            "Q > 0 (yellow) = weak, Q ≤ 0 (red) = worse than random.",
            "Sparkline shows relative circuit sizes (█ = largest, ▁ = smallest).",
            "Sizes: min–max (avg) of functions per circuit.",
        ]);
    }

    match circuits::circuit_summary(rs_files, project_path_canon) {
        Some(cs) => render_circuit_overview(&cs),
        None => println!("  {}", "No circuit data available.".dimmed()),
    }
}

/// Render the statistics footer with aggregate metrics.
fn render_stats_footer(
    agg_loc: &Option<loc::AggregateStats>,
    complexity_stats: &Option<complexity::ComplexityStats>,
    total_files: usize,
    total_functions: usize,
    total_modules: usize,
    verbose: bool,
) {
    render::section_header("Statistics");

    if verbose {
        render::verbose_block(&[
            "Aggregate statistics across all analyzed source files and functions.",
            "LoC mean / std = average and standard deviation of code lines per file.",
            "LoC median = middle value when files are sorted by size (robust to outliers).",
            "Complexity mean / std = average and spread of composite complexity scores.",
            "Complexity median = middle composite score across all functions.",
        ]);
    }

    let box_w = 42;
    println!("  ┌{}┐", "─".repeat(box_w));

    if let Some(agg) = agg_loc {
        print_stat(
            "LoC mean / std",
            &format!("{:.1} / {:.1}", agg.mean, agg.std_dev),
            box_w,
        );
        print_stat("LoC median", &format!("{}", agg.median), box_w);
    }
    if let Some(cs) = complexity_stats {
        print_stat(
            "Complexity mean / std",
            &format!("{:.1} / {:.1}", cs.mean, cs.std_dev),
            box_w,
        );
        print_stat("Complexity median", &format!("{:.1}", cs.median), box_w);
    }
    print_stat("Total files", &format!("{}", total_files), box_w);
    print_stat("Total functions", &format!("{}", total_functions), box_w);
    print_stat("Total modules", &format!("{}", total_modules), box_w);

    println!("  └{}┘", "─".repeat(box_w));
    println!();
}

// ── JSON Output ──────────────────────────────────────────────────

#[derive(Serialize)]
struct SummaryLocJson {
    total_files: usize,
    total_loc: usize,
    mean: f64,
    std_dev: f64,
    median: usize,
    min: usize,
    max: usize,
}

#[derive(Serialize)]
struct SummaryComplexityJson {
    total_functions: usize,
    mean: f64,
    std_dev: f64,
    median: f64,
    max: f64,
}

#[derive(Serialize)]
struct SummaryDepsJson {
    total_modules: usize,
    total_edges: usize,
    bidirectional_pairs: usize,
    highest_fan_in: Option<(String, usize)>,
    highest_fan_out: Option<(String, usize)>,
}

#[derive(Serialize)]
struct SummaryGraphJson {
    top_bridge: Option<(String, f64)>,
    top_hub: Option<(String, f64)>,
    top_clustered: Option<(String, f64)>,
}

#[derive(Serialize)]
struct SummaryReachabilityJson {
    reachable_pct: f64,
    total_functions: usize,
    reachable_count: usize,
    cold_count: usize,
}

#[derive(Serialize)]
struct SummaryCircuitsJson {
    circuit_count: usize,
    sizes: Vec<usize>,
    modularity: f64,
}

#[derive(Serialize)]
struct SummaryJson {
    cstat_version: String,
    project: String,
    loc: Option<SummaryLocJson>,
    complexity: Option<SummaryComplexityJson>,
    deps: SummaryDepsJson,
    graph: Option<SummaryGraphJson>,
    reachability: Option<SummaryReachabilityJson>,
    circuits: Option<SummaryCircuitsJson>,
}

/// Render the summary as JSON.
pub fn render_summary_json(rs_files: &[PathBuf], project_path: &Path) {
    let project_path_canon = project_path
        .canonicalize()
        .unwrap_or_else(|_| project_path.to_path_buf());

    let project_name = project_path_canon
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let (_file_stats, agg_loc) = loc::analyze_files(rs_files);
    let symbols = ast_parser::parse_project(rs_files);
    let func_complexities = complexity::compute_all(&symbols, &project_path_canon);
    let complexity_stats = complexity::compute_stats(&func_complexities);
    let dep_analysis = deps::analyze_deps(rs_files, &project_path_canon);

    let loc_json = agg_loc.map(|a| SummaryLocJson {
        total_files: a.total_files,
        total_loc: a.total_loc,
        mean: a.mean,
        std_dev: a.std_dev,
        median: a.median,
        min: a.min,
        max: a.max,
    });

    let complexity_json = complexity_stats.map(|s| SummaryComplexityJson {
        total_functions: s.total_functions,
        mean: s.mean,
        std_dev: s.std_dev,
        median: s.median,
        max: s.max,
    });

    let mut by_fan_in: Vec<(&String, usize)> = dep_analysis
        .in_degree
        .iter()
        .map(|(m, d)| (m, *d))
        .filter(|(_, d)| *d > 0)
        .collect();
    by_fan_in.sort_by(|a, b| b.1.cmp(&a.1));
    let mut by_fan_out: Vec<(&String, usize)> = dep_analysis
        .out_degree
        .iter()
        .map(|(m, d)| (m, *d))
        .filter(|(_, d)| *d > 0)
        .collect();
    by_fan_out.sort_by(|a, b| b.1.cmp(&a.1));

    let bidi_count = dep_analysis
        .coupling_pairs
        .iter()
        .filter(|(_, _, s)| *s >= 2)
        .count();

    let deps_json = SummaryDepsJson {
        total_modules: dep_analysis.modules.len(),
        total_edges: dep_analysis.edges.len(),
        bidirectional_pairs: bidi_count,
        highest_fan_in: by_fan_in.first().map(|(n, d)| ((*n).clone(), *d)),
        highest_fan_out: by_fan_out.first().map(|(n, d)| ((*n).clone(), *d)),
    };

    let graph_json =
        graph::graph_summary(rs_files, &project_path_canon).map(|gs| SummaryGraphJson {
            top_bridge: gs.top_bridge,
            top_hub: gs.top_hub,
            top_clustered: gs.top_clustered,
        });

    let reachability_json =
        datapaths::entrypoint_reachability_summary(rs_files, &project_path_canon).map(|ds| {
            SummaryReachabilityJson {
                reachable_pct: ds.reachable_pct,
                total_functions: ds.total_functions,
                reachable_count: ds.reachable_count,
                cold_count: ds.cold_count,
            }
        });

    let circuits_json =
        circuits::circuit_summary(rs_files, &project_path_canon).map(|cs| SummaryCircuitsJson {
            circuit_count: cs.circuit_count,
            sizes: cs.sizes,
            modularity: cs.modularity,
        });

    let output = SummaryJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        project: project_name,
        loc: loc_json,
        complexity: complexity_json,
        deps: deps_json,
        graph: graph_json,
        reachability: reachability_json,
        circuits: circuits_json,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}
