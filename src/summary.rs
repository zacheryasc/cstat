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

    // ── Header ───────────────────────────────────────────────
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

    // ── LoC Chart (compact top-10) ───────────────────────────
    render_section_header("Lines of Code — Top Files", term_w);

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
        let display_count = file_stats.len().min(10);
        let display_stats = &file_stats[..display_count];
        let max_label_len = display_stats
            .iter()
            .map(|f| short_path(&f.path, &project_path_canon).len())
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
            .saturating_sub(max_label_len + 2) // label + padding
            .saturating_sub(3)                  // " │ "
            .saturating_sub(loc_digits)
            .saturating_sub(2)
            .max(10);

        for (i, stat) in display_stats.iter().enumerate() {
            let ratio = i as f64 / display_count.max(1) as f64;
            let label = short_path_truncated(&stat.path, &project_path_canon, max_label_len);
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

        // Directory proportion breakdown
        if verbose {
            println!();
            render::verbose_block(&[
                "Directory breakdown: proportional LoC by top-level directory.",
                "Bar size = share of total code. Color: red > 50%, yellow > 25%, green otherwise.",
            ]);
        }
        render_dir_breakdown(&file_stats, &project_path_canon, term_w, total_loc);
    }

    // ── Complexity Hotspots (top 10) ─────────────────────────
    render_section_header("Complexity Hotspots", term_w);

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

    // ── Module Connectome Overview ───────────────────────────
    render_section_header("Module Connectome", term_w);

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
        // Mini dependency heatmap — compact visual matrix
        render_mini_heatmap(&dep_analysis, term_w);

        // Most coupled pairs
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

        // Highest fan-in / fan-out
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

    // ── Sparkline Distributions ─────────────────────────────
    render_section_header("Metric Distributions", term_w);

    if verbose {
        render::verbose_block(&[
            "Sparklines show the sorted distribution shape for each metric.",
            "Values are sorted ascending, then binned into ~40 buckets to form the sparkline.",
            "▁ = low, █ = high. A flat line means uniform values; a steep ramp means outliers.",
            "Range shows min–max. μ = arithmetic mean across all functions.",
        ]);
    }

    if !func_complexities.is_empty() {
        render_sparkline_distributions(&func_complexities);
    } else {
        println!("  {}", "No functions to analyze.".dimmed());
    }

    // ── Graph Centrality Highlights ──────────────────────────
    render_section_header("Graph Centrality", term_w);

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

    match graph::graph_summary(rs_files, &project_path_canon) {
        Some(gs) => render_graph_centrality(&gs),
        None => println!("  {}", "No call graph data available.".dimmed()),
    }

    // ── Datapath Coverage ────────────────────────────────────
    render_section_header("Datapath Coverage", term_w);

    if verbose {
        render::verbose_block(&[
            "Percentage of functions reachable from entry points via call graph traversal.",
            "Entry points are main() and #[test] functions. Reachability is computed via BFS.",
            "Coverage = reachable / total. Cold functions = unreachable = dead code candidates.",
            "Color: green > 80%, yellow > 50%, red ≤ 50%.",
        ]);
    }

    match datapaths::datapath_summary(rs_files, &project_path_canon) {
        Some(ds) => render_datapath_coverage(&ds),
        None => println!("  {}", "No datapath data available.".dimmed()),
    }

    // ── Circuit Overview ─────────────────────────────────────
    render_section_header("Circuits", term_w);

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

    match circuits::circuit_summary(rs_files, &project_path_canon) {
        Some(cs) => render_circuit_overview(&cs),
        None => println!("  {}", "No circuit data available.".dimmed()),
    }

    // ── Stats Footer ─────────────────────────────────────────
    render_section_header("Statistics", term_w);

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

    if let Some(agg) = &agg_loc {
        print_stat("LoC mean / std", &format!("{:.1} / {:.1}", agg.mean, agg.std_dev), box_w);
        print_stat("LoC median", &format!("{}", agg.median), box_w);
    }
    if let Some(cs) = &complexity_stats {
        print_stat("Complexity mean / std", &format!("{:.1} / {:.1}", cs.mean, cs.std_dev), box_w);
        print_stat("Complexity median", &format!("{:.1}", cs.median), box_w);
    }
    print_stat("Total files", &format!("{}", total_files), box_w);
    print_stat("Total functions", &format!("{}", total_functions), box_w);
    print_stat("Total modules", &format!("{}", total_modules), box_w);

    println!("  └{}┘", "─".repeat(box_w));
    println!();
}

// ── New summary sections ─────────────────────────────────────

/// Build a sparkline string from a slice of f64 values.
fn sparkline(values: &[f64]) -> String {
    if values.is_empty() {
        return String::new();
    }
    let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = (max - min).max(f64::EPSILON);

    // Bin values into ~40 buckets for a compact sparkline
    let num_bins = 40.min(values.len());
    let bin_size = (values.len() as f64 / num_bins as f64).ceil() as usize;

    let mut bins: Vec<f64> = Vec::new();
    for chunk in values.chunks(bin_size) {
        let avg = chunk.iter().sum::<f64>() / chunk.len() as f64;
        bins.push(avg);
    }

    bins.iter()
        .map(|&v| {
            let idx = ((v - min) / range * 7.0).round() as usize;
            blocks[idx.min(7)]
        })
        .collect()
}

/// Render single-line sparkline distributions for key metrics.
fn render_sparkline_distributions(fcs: &[complexity::FunctionComplexity]) {
    let metrics: Vec<(&str, Vec<f64>)> = vec![
        ("Function LoC", fcs.iter().map(|f| f.line_count as f64).collect()),
        ("Cyclomatic", fcs.iter().map(|f| f.cyclomatic as f64).collect()),
        ("Cognitive", fcs.iter().map(|f| f.cognitive as f64).collect()),
        ("Nesting", fcs.iter().map(|f| f.nesting_depth as f64).collect()),
        ("Params", fcs.iter().map(|f| f.param_count as f64).collect()),
    ];

    for (name, mut values) in metrics {
        if values.is_empty() {
            continue;
        }
        // Sort values so sparkline shows the distribution shape (ascending)
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let spark = sparkline(&values);
        let min = values.first().copied().unwrap_or(0.0);
        let max = values.last().copied().unwrap_or(0.0);
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        println!(
            "  {:<14} {} {:.0}–{:.0} (μ={:.1})",
            name.dimmed(),
            spark.bright_yellow(),
            min,
            max,
            mean,
        );
    }
}

/// Render graph centrality highlights.
fn render_graph_centrality(gs: &graph::GraphSummary) {
    if let Some((name, val)) = &gs.top_bridge {
        let short = if name.len() > 30 { &name[name.len() - 30..] } else { name.as_str() };
        println!(
            "  {} {} (bc={:.4})",
            "Top bridge:".bold(),
            short.bright_blue(),
            val,
        );
    }
    if let Some((name, val)) = &gs.top_hub {
        let short = if name.len() > 30 { &name[name.len() - 30..] } else { name.as_str() };
        println!(
            "  {} {} (pr={:.4})",
            "Top hub:".bold(),
            short.yellow(),
            val,
        );
    }
    if let Some((name, val)) = &gs.top_clustered {
        let short = if name.len() > 30 { &name[name.len() - 30..] } else { name.as_str() };
        println!(
            "  {} {} (cc={:.4})",
            "Most clustered:".bold(),
            short.green(),
            val,
        );
    }
    if gs.top_bridge.is_none() && gs.top_hub.is_none() && gs.top_clustered.is_none() {
        println!("  {}", "No centrality data available.".dimmed());
    }
}

/// Render datapath coverage section.
fn render_datapath_coverage(ds: &datapaths::DatapathSummary) {
    let coverage_color = if ds.coverage_pct > 80.0 {
        format!("{:.1}%", ds.coverage_pct).green().bold().to_string()
    } else if ds.coverage_pct > 50.0 {
        format!("{:.1}%", ds.coverage_pct).yellow().bold().to_string()
    } else {
        format!("{:.1}%", ds.coverage_pct).red().bold().to_string()
    };
    println!(
        "  {} {} reachable from entry points",
        "Coverage:".bold(),
        coverage_color,
    );
    println!(
        "  {} reachable, {} cold (of {} total)",
        format!("{}", ds.reachable_count).bold(),
        if ds.cold_count > 0 {
            format!("{}", ds.cold_count).red().to_string()
        } else {
            format!("{}", ds.cold_count).green().to_string()
        },
        ds.total_functions,
    );
}

/// Render circuit overview section.
fn render_circuit_overview(cs: &circuits::CircuitSummary) {
    let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let size_spark: String = if cs.sizes.len() > 1 {
        let max_s = *cs.sizes.iter().max().unwrap_or(&1) as f64;
        cs.sizes
            .iter()
            .map(|&s| {
                let idx = ((s as f64 / max_s) * 7.0).round() as usize;
                blocks[idx.min(7)]
            })
            .collect()
    } else if cs.sizes.len() == 1 {
        "█".to_string()
    } else {
        String::new()
    };

    println!(
        "  {} {} circuit{}  {}",
        "Circuits:".bold(),
        format!("{}", cs.circuit_count).bold(),
        if cs.circuit_count == 1 { "" } else { "s" },
        size_spark.bright_yellow(),
    );

    let mod_color = if cs.modularity > 0.3 {
        format!("{:.4}", cs.modularity).green().bold().to_string()
    } else if cs.modularity > 0.0 {
        format!("{:.4}", cs.modularity).yellow().to_string()
    } else {
        format!("{:.4}", cs.modularity).red().to_string()
    };
    println!(
        "  {} {}",
        "Modularity:".bold(),
        mod_color,
    );

    if !cs.sizes.is_empty() {
        let max_s = cs.sizes.iter().max().unwrap_or(&0);
        let min_s = cs.sizes.iter().min().unwrap_or(&0);
        let avg_s = cs.sizes.iter().sum::<usize>() as f64 / cs.sizes.len() as f64;
        println!(
            "  Sizes: {}–{} (avg {:.1})",
            min_s, max_s, avg_s,
        );
    }
}

// ── Helpers ──────────────────────────────────────────────────

fn render_section_header(title: &str, _term_w: usize) {
    println!(
        "\n┌─ {} {}",
        title.bright_cyan().bold(),
        "─".repeat(50usize.saturating_sub(title.len() + 3)).bright_cyan(),
    );
}

fn print_stat(label: &str, value: &str, box_w: usize) {
    let inner = box_w - 2; // accounting for padding spaces
    let val_w = inner.saturating_sub(label.len() + 3);
    println!(
        "  │ {} {:>w$} │",
        label.cyan(),
        value.bold(),
        w = val_w,
    );
}

fn short_path(path: &Path, base: &Path) -> String {
    let rel = path.strip_prefix(base).unwrap_or(path);
    rel.display().to_string()
}

fn short_path_truncated(path: &Path, base: &Path, max_len: usize) -> String {
    let s = short_path(path, base);
    if s.len() > max_len {
        format!("...{}", &s[s.len() - (max_len - 3)..])
    } else {
        s
    }
}

/// Render a compact mini heatmap matrix in the summary's connectome section.
fn render_mini_heatmap(analysis: &deps::DepAnalysis, term_w: usize) {
    use std::collections::HashSet;

    // Pick top modules by connectivity (cap at 8 for compactness)
    let mut module_connectivity: Vec<(&String, usize)> = analysis
        .modules
        .iter()
        .map(|m| {
            let in_d = analysis.in_degree.get(m).copied().unwrap_or(0);
            let out_d = analysis.out_degree.get(m).copied().unwrap_or(0);
            (m, in_d + out_d)
        })
        .collect();
    module_connectivity.sort_by(|a, b| b.1.cmp(&a.1));

    let display_modules: Vec<&String> = module_connectivity
        .iter()
        .filter(|(_, c)| *c > 0)
        .take(8)
        .map(|(m, _)| *m)
        .collect();

    if display_modules.is_empty() {
        return;
    }

    let edge_set: HashSet<(&str, &str)> = analysis
        .edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();

    let n = display_modules.len();

    // Abbreviated module names: take last path segment
    let short_names: Vec<&str> = display_modules
        .iter()
        .map(|m| m.rsplit("::").next().unwrap_or(m))
        .collect();
    let max_short = short_names.iter().map(|s| s.len()).max().unwrap_or(3).min(12);

    // Check if matrix fits in terminal
    let matrix_cols = max_short + 3 + n * 2 + 4;
    if matrix_cols > term_w {
        // Too wide, skip visual matrix
        return;
    }

    // Column header indices
    print!("  {:>width$}   ", "", width = max_short);
    for i in 0..n {
        print!("{} ", format!("{}", i).dimmed());
    }
    println!();

    // Matrix rows
    for (ri, row_mod) in display_modules.iter().enumerate() {
        let label = if short_names[ri].len() > max_short {
            &short_names[ri][..max_short]
        } else {
            short_names[ri]
        };
        print!("  {:>width$} │ ", label.dimmed(), width = max_short);

        for (ci, col_mod) in display_modules.iter().enumerate() {
            if ri == ci {
                print!("{} ", "·".dimmed());
            } else {
                let fwd = edge_set.contains(&(row_mod.as_str(), col_mod.as_str()));
                let rev = edge_set.contains(&(col_mod.as_str(), row_mod.as_str()));
                let strength = (if fwd { 1 } else { 0 }) + (if rev { 1 } else { 0 });
                match strength {
                    0 => print!("{} ", "0".dimmed()),
                    1 => print!("{} ", "1".yellow()),
                    _ => print!("{} ", "2".red().bold()),
                }
            }
        }
        println!("{}", format!("{}", ri).dimmed());
    }

    // Compact legend
    println!(
        "  {} {} {}",
        "0".dimmed(),
        "1".yellow(),
        "2".red().bold(),
    );
    println!();
}

/// Render a proportional directory breakdown showing LoC distribution by directory.
fn render_dir_breakdown(
    file_stats: &[loc::FileLocStats],
    project_path: &Path,
    term_w: usize,
    total_loc: usize,
) {
    use std::collections::BTreeMap;

    if total_loc == 0 {
        return;
    }

    // Aggregate LoC by top-level directory
    let mut dir_loc: BTreeMap<String, usize> = BTreeMap::new();
    for stat in file_stats {
        let rel = stat
            .path
            .strip_prefix(project_path)
            .unwrap_or(&stat.path);
        let components: Vec<_> = rel.components().collect();
        let dir_name = if components.len() <= 1 {
            // File in root
            ".".to_string()
        } else {
            components[0].as_os_str().to_string_lossy().to_string()
        };
        *dir_loc.entry(dir_name).or_insert(0) += stat.code_lines;
    }

    if dir_loc.is_empty() {
        return;
    }

    // Sort by LoC descending
    let mut dirs: Vec<(String, usize)> = dir_loc.into_iter().collect();
    dirs.sort_by(|a, b| b.1.cmp(&a.1));

    println!();
    println!("  {}", "Directory breakdown:".bold());

    let max_dir_label = dirs.iter().map(|(d, _)| d.len()).max().unwrap_or(3).min(20);
    let bar_budget = term_w
        .saturating_sub(max_dir_label + 4) // "  " + label + " "
        .saturating_sub(12) // percentage + loc count
        .max(10);

    for (dir, loc) in &dirs {
        let pct = *loc as f64 / total_loc as f64 * 100.0;
        let bar_len = (*loc as f64 / total_loc as f64 * bar_budget as f64).ceil() as usize;
        let bar_len = bar_len.max(if *loc > 0 { 1 } else { 0 });

        let ratio = *loc as f64 / total_loc as f64;
        let color_fn = |t: &str| -> String {
            if ratio > 0.5 {
                t.red().bold().to_string()
            } else if ratio > 0.25 {
                t.yellow().to_string()
            } else {
                t.green().to_string()
            }
        };

        let bar = color_fn(&"█".repeat(bar_len));
        let padding = " ".repeat(bar_budget.saturating_sub(bar_len));
        let label = if dir.len() > max_dir_label {
            format!("..{}", &dir[dir.len() - (max_dir_label - 2)..])
        } else {
            format!("{}/", dir)
        };

        println!(
            "  {:>width$} {}{} {:>5.1}%",
            label.dimmed(),
            bar,
            padding,
            pct,
            width = max_dir_label + 1,
        );
    }
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
struct SummaryDatapathJson {
    coverage_pct: f64,
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
    datapaths: Option<SummaryDatapathJson>,
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
        .in_degree.iter().map(|(m, d)| (m, *d)).filter(|(_, d)| *d > 0).collect();
    by_fan_in.sort_by(|a, b| b.1.cmp(&a.1));
    let mut by_fan_out: Vec<(&String, usize)> = dep_analysis
        .out_degree.iter().map(|(m, d)| (m, *d)).filter(|(_, d)| *d > 0).collect();
    by_fan_out.sort_by(|a, b| b.1.cmp(&a.1));

    let bidi_count = dep_analysis.coupling_pairs.iter().filter(|(_, _, s)| *s >= 2).count();

    let deps_json = SummaryDepsJson {
        total_modules: dep_analysis.modules.len(),
        total_edges: dep_analysis.edges.len(),
        bidirectional_pairs: bidi_count,
        highest_fan_in: by_fan_in.first().map(|(n, d)| ((*n).clone(), *d)),
        highest_fan_out: by_fan_out.first().map(|(n, d)| ((*n).clone(), *d)),
    };

    let graph_json = graph::graph_summary(rs_files, &project_path_canon).map(|gs| SummaryGraphJson {
        top_bridge: gs.top_bridge,
        top_hub: gs.top_hub,
        top_clustered: gs.top_clustered,
    });

    let datapaths_json = datapaths::datapath_summary(rs_files, &project_path_canon).map(|ds| SummaryDatapathJson {
        coverage_pct: ds.coverage_pct,
        total_functions: ds.total_functions,
        reachable_count: ds.reachable_count,
        cold_count: ds.cold_count,
    });

    let circuits_json = circuits::circuit_summary(rs_files, &project_path_canon).map(|cs| SummaryCircuitsJson {
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
        datapaths: datapaths_json,
        circuits: circuits_json,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}
