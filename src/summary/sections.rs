use colored::Colorize;
use std::collections::HashSet;
use std::path::Path;

use crate::circuits;
use crate::complexity;
use crate::datapaths;
use crate::deps;
use crate::graph;
use crate::loc;

/// Build a sparkline string from a slice of f64 values.
pub(super) fn sparkline(values: &[f64]) -> String {
    if values.is_empty() {
        return String::new();
    }
    let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = (max - min).max(f64::EPSILON);

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
pub(super) fn render_sparkline_distributions(fcs: &[complexity::FunctionComplexity]) {
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
pub(super) fn render_graph_centrality(gs: &graph::GraphSummary) {
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
pub(super) fn render_datapath_coverage(ds: &datapaths::DatapathSummary) {
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
pub(super) fn render_circuit_overview(cs: &circuits::CircuitSummary) {
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

pub(super) fn print_stat(label: &str, value: &str, box_w: usize) {
    let inner = box_w - 2;
    let val_w = inner.saturating_sub(label.len() + 3);
    println!(
        "  │ {} {:>w$} │",
        label.cyan(),
        value.bold(),
        w = val_w,
    );
}

pub(super) fn short_path(path: &Path, base: &Path) -> String {
    let rel = path.strip_prefix(base).unwrap_or(path);
    rel.display().to_string()
}

pub(super) fn short_path_truncated(path: &Path, base: &Path, max_len: usize) -> String {
    let s = short_path(path, base);
    if s.len() > max_len {
        format!("...{}", &s[s.len() - (max_len - 3)..])
    } else {
        s
    }
}

/// Render a compact mini heatmap matrix in the summary's connectome section.
pub(super) fn render_mini_heatmap(analysis: &deps::DepAnalysis, term_w: usize) {
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

    let short_names: Vec<&str> = display_modules
        .iter()
        .map(|m| m.rsplit("::").next().unwrap_or(m))
        .collect();
    let max_short = short_names.iter().map(|s| s.len()).max().unwrap_or(3).min(12);

    let matrix_cols = max_short + 3 + n * 2 + 4;
    if matrix_cols > term_w {
        return;
    }

    print!("  {:>width$}   ", "", width = max_short);
    for i in 0..n {
        print!("{} ", format!("{}", i).dimmed());
    }
    println!();

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

    println!(
        "  {} {} {}",
        "0".dimmed(),
        "1".yellow(),
        "2".red().bold(),
    );
    println!();
}

/// Render a proportional directory breakdown showing LoC distribution by directory.
pub(super) fn render_dir_breakdown(
    file_stats: &[loc::FileLocStats],
    project_path: &Path,
    term_w: usize,
    total_loc: usize,
) {
    use std::collections::BTreeMap;

    if total_loc == 0 {
        return;
    }

    let mut dir_loc: BTreeMap<String, usize> = BTreeMap::new();
    for stat in file_stats {
        let rel = stat
            .path
            .strip_prefix(project_path)
            .unwrap_or(&stat.path);
        let components: Vec<_> = rel.components().collect();
        let dir_name = if components.len() <= 1 {
            ".".to_string()
        } else {
            components[0].as_os_str().to_string_lossy().to_string()
        };
        *dir_loc.entry(dir_name).or_insert(0) += stat.code_lines;
    }

    if dir_loc.is_empty() {
        return;
    }

    let mut dirs: Vec<(String, usize)> = dir_loc.into_iter().collect();
    dirs.sort_by(|a, b| b.1.cmp(&a.1));

    println!();
    println!("  {}", "Directory breakdown:".bold());

    let max_dir_label = dirs.iter().map(|(d, _)| d.len()).max().unwrap_or(3).min(20);
    let bar_budget = term_w
        .saturating_sub(max_dir_label + 4)
        .saturating_sub(12)
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
