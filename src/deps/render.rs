use colored::Colorize;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::render;

use super::{DepAnalysis, ModuleCohesion, analyze_deps, compute_cohesion};

#[derive(Serialize)]
struct DepEdgeJson {
    from: String,
    to: String,
}

#[derive(Serialize)]
struct ModuleDegreeJson {
    module: String,
    in_degree: usize,
    out_degree: usize,
}

#[derive(Serialize)]
struct CouplingPairJson {
    module_a: String,
    module_b: String,
    strength: usize,
    direction: String,
}

#[derive(Serialize)]
struct DepsJson {
    cstat_version: String,
    modules: Vec<String>,
    edges: Vec<DepEdgeJson>,
    module_degrees: Vec<ModuleDegreeJson>,
    coupling_pairs: Vec<CouplingPairJson>,
    cohesion: Vec<ModuleCohesion>,
}

/// Render the deps subcommand output.
pub fn render_deps(rs_files: &[PathBuf], project_path: &Path, top_n: Option<usize>, verbose: bool) {
    let analysis = analyze_deps(rs_files, project_path);

    if analysis.modules.is_empty() {
        println!("{}", "No modules found to analyze.".yellow());
        return;
    }

    let term_w = render::terminal_width();
    let top = top_n.unwrap_or(15);

    render_dependency_matrix(&analysis, top, term_w, verbose);
    render_coupling_ranking(&analysis, top, verbose);
    render_fan_chart(&analysis, top, term_w, verbose);

    let cohesion = compute_cohesion(rs_files, project_path);
    render_cohesion_section(&cohesion, top, verbose);
}

/// Render the column header row for the dependency matrix.
fn render_matrix_header(display_modules: &[&String], max_label: usize, col_width: usize) {
    println!();
    print!("{:>width$} │ ", "", width = max_label);
    for (i, _) in display_modules.iter().enumerate() {
        print!("{:>w$}", format!("{}", i), w = col_width);
    }
    println!();

    print!("{}", "─".repeat(max_label + 1));
    print!("┼");
    println!("{}", "─".repeat(display_modules.len() * col_width + 1));
}

/// Render the data rows of the dependency matrix.
fn render_matrix_rows(
    display_modules: &[&String],
    edge_set: &HashSet<(&str, &str)>,
    max_label: usize,
    col_width: usize,
) {
    for (ri, row_mod) in display_modules.iter().enumerate() {
        let label = if row_mod.len() > max_label {
            format!("..{}", &row_mod[row_mod.len() - (max_label - 2)..])
        } else {
            (*row_mod).clone()
        };
        print!("{:>width$} │ ", label.dimmed(), width = max_label);

        for col_mod in display_modules {
            if *row_mod == *col_mod {
                let cell = "·".dimmed().to_string();
                let visible_len = 1;
                let pad = col_width.saturating_sub(visible_len);
                print!("{}{}", " ".repeat(pad), cell);
            } else {
                let has_fwd = edge_set.contains(&(row_mod.as_str(), col_mod.as_str()));
                let has_rev = edge_set.contains(&(col_mod.as_str(), row_mod.as_str()));
                let strength = (if has_fwd { 1 } else { 0 }) + (if has_rev { 1 } else { 0 });

                let cell = match strength {
                    0 => "0".dimmed().to_string(),
                    1 => "1".yellow().to_string(),
                    _ => "2".red().bold().to_string(),
                };
                let visible_len = 1;
                let pad = col_width.saturating_sub(visible_len);
                print!("{}{}", " ".repeat(pad), cell);
            }
        }
        println!("  {}", format!("{}", ri).dimmed());
    }
}

/// Render the legend below the dependency matrix.
fn render_matrix_legend(display_modules: &[&String]) {
    println!();
    println!(
        "  {} no dependency  {} unidirectional  {} bidirectional",
        "0".dimmed(),
        "1".yellow(),
        "2".red().bold(),
    );

    println!();
    for (i, m) in display_modules.iter().enumerate() {
        println!("  {} = {}", format!("{:>2}", i).bold(), m.dimmed());
    }
}

fn render_dependency_matrix(analysis: &DepAnalysis, top_n: usize, term_w: usize, verbose: bool) {
    println!(
        "\n{}",
        "── Dependency Heatmap ─────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Matrix showing coupling intensity between module pairs.",
            "Cell [row, col] = 0 (dim, no dependency), 1 (yellow, unidirectional),",
            "  2 (red/bold, bidirectional — both modules import each other).",
            "Diagonal is marked '·' (a module cannot depend on itself).",
            "Modules are sorted by total connectivity (in + out degree).",
            "Row/column indices map to module names shown in the legend below the matrix.",
            "Built from 'use' and 'mod' statements found by parsing each file's AST.",
        ]);
        render::guide_ref("modularity");
    }

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
        .take(top_n)
        .map(|(m, _)| *m)
        .collect();

    if display_modules.is_empty() {
        println!("  {}", "No inter-module dependencies detected.".dimmed());
        return;
    }

    let edge_set: HashSet<(&str, &str)> = analysis
        .edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();

    let max_label = 20.min(
        display_modules
            .iter()
            .map(|m| m.len())
            .max()
            .unwrap_or(5),
    );

    let matrix_start = max_label + 3;
    let available = term_w.saturating_sub(matrix_start);
    let col_width = 3.min(available / display_modules.len().max(1)).max(2);

    let max_cols = available / col_width.max(1);
    let display_count = display_modules.len().min(max_cols);
    let display_modules = &display_modules[..display_count];

    render_matrix_header(display_modules, max_label, col_width);
    render_matrix_rows(display_modules, &edge_set, max_label, col_width);
    render_matrix_legend(display_modules);
}

fn render_coupling_ranking(analysis: &DepAnalysis, top_n: usize, verbose: bool) {
    println!(
        "\n{}",
        "── Coupling Ranking ───────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Module pairs ranked by coupling strength.",
            "◆ bidirectional (red) = both modules import each other. This is a coupling risk —",
            "  changes in either module may ripple to the other.",
            "→ unidirectional (yellow) = one-way dependency, lower risk.",
            "Pairs are sorted by coupling strength descending, then alphabetically.",
        ]);
    }

    if analysis.coupling_pairs.is_empty() {
        println!("  {}", "No module coupling detected.".dimmed());
        return;
    }

    let display_count = analysis.coupling_pairs.len().min(top_n);
    println!(
        "\n  {:<30} {:<30} {}",
        "Module A".bold().underline(),
        "Module B".bold().underline(),
        "Coupling".bold().underline(),
    );

    for (a, b, strength) in analysis.coupling_pairs.iter().take(display_count) {
        let coupling_label = if *strength >= 2 {
            "◆ bidirectional".red().bold().to_string()
        } else {
            "→ unidirectional".yellow().to_string()
        };

        let a_display = if a.len() > 28 {
            format!("..{}", &a[a.len() - 26..])
        } else {
            a.clone()
        };
        let b_display = if b.len() > 28 {
            format!("..{}", &b[b.len() - 26..])
        } else {
            b.clone()
        };

        println!("  {:<30} {:<30} {}", a_display.dimmed(), b_display.dimmed(), coupling_label);
    }

    if analysis.coupling_pairs.len() > display_count {
        println!(
            "{}",
            format!(
                "  ... and {} more pairs",
                analysis.coupling_pairs.len() - display_count
            )
            .dimmed()
        );
    }
}

/// Render the bar chart rows for fan-in / fan-out.
fn render_fan_bars(
    display_stats: &[(&String, usize, usize)],
    max_label_len: usize,
    max_degree: usize,
    bar_budget: usize,
) {
    for (name, in_d, out_d) in display_stats {
        let label = if name.len() > max_label_len {
            format!("..{}", &name[name.len() - (max_label_len - 2)..])
        } else {
            (*name).clone()
        };

        let total = in_d + out_d;
        let total_bar = if max_degree > 0 {
            (total as f64 / max_degree as f64 * bar_budget as f64).ceil() as usize
        } else {
            0
        }
        .max(if total > 0 { 1 } else { 0 });

        let in_bar = if total > 0 {
            ((*in_d as f64 / total as f64) * total_bar as f64).round() as usize
        } else {
            0
        };
        let out_bar = total_bar.saturating_sub(in_bar);

        let in_str = "█".repeat(in_bar).bright_blue().to_string();
        let out_str = "░".repeat(out_bar).yellow().to_string();
        let padding = " ".repeat(bar_budget.saturating_sub(total_bar));

        println!(
            "{:>width$} │ {}{}{} in:{} out:{}",
            label.dimmed(),
            in_str,
            out_str,
            padding,
            format!("{}", in_d).bright_blue(),
            format!("{}", out_d).yellow(),
            width = max_label_len,
        );
    }
}

fn render_fan_chart(analysis: &DepAnalysis, top_n: usize, term_w: usize, verbose: bool) {
    println!(
        "\n{}",
        "── Fan-in / Fan-out ───────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Horizontal bar chart showing each module's connectivity profile.",
            "█ (blue) = fan-in: how many modules depend on this one.",
            "  High fan-in = core utility — many other modules import it.",
            "░ (yellow) = fan-out: how many modules this one depends on.",
            "  High fan-out = potential god module — it knows about everything.",
            "Modules sorted by total connectivity (fan-in + fan-out) descending.",
        ]);
    }

    let mut module_stats: Vec<(&String, usize, usize)> = analysis
        .modules
        .iter()
        .map(|m| {
            let in_d = analysis.in_degree.get(m).copied().unwrap_or(0);
            let out_d = analysis.out_degree.get(m).copied().unwrap_or(0);
            (m, in_d, out_d)
        })
        .filter(|(_, i, o)| *i > 0 || *o > 0)
        .collect();

    module_stats.sort_by(|a, b| (b.1 + b.2).cmp(&(a.1 + a.2)));

    if module_stats.is_empty() {
        println!("  {}", "No inter-module dependencies detected.".dimmed());
        return;
    }

    let display_count = module_stats.len().min(top_n);
    let display_stats = &module_stats[..display_count];

    let max_label_len = display_stats
        .iter()
        .map(|(m, _, _)| m.len())
        .max()
        .unwrap_or(5)
        .min(25);

    let max_degree = display_stats
        .iter()
        .map(|(_, i, o)| *i + *o)
        .max()
        .unwrap_or(1);

    let suffix_len = 14;
    let bar_budget = term_w
        .saturating_sub(max_label_len)
        .saturating_sub(3)
        .saturating_sub(suffix_len)
        .max(10);

    println!(
        "\n{:>width$}   {}{}",
        "",
        "█ fan-in".bright_blue(),
        "  ░ fan-out".yellow(),
        width = max_label_len,
    );
    println!();

    render_fan_bars(display_stats, max_label_len, max_degree, bar_budget);

    if module_stats.len() > display_count {
        println!(
            "{}",
            format!(
                "  ... and {} more modules",
                module_stats.len() - display_count
            )
            .dimmed()
        );
    }

    println!();
}

fn render_cohesion_section(cohesion: &[ModuleCohesion], top_n: usize, verbose: bool) {
    println!(
        "\n{}",
        "── Module Cohesion ───────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Per-module cohesion scores measuring internal focus/coherence.",
            "Call cohesion: fraction of functions calling another function in the same module.",
            "Type cohesion: fraction of functions sharing parameter/return types within the module.",
            "Combined: weighted average (60% call + 40% type), 0.0–1.0 where higher = more cohesive.",
            "Low cohesion suggests the module bundles unrelated functionality.",
            "Modules sorted by combined cohesion ascending (least cohesive first).",
        ]);
    }

    let display: Vec<&ModuleCohesion> = cohesion.iter()
        .filter(|c| c.function_count > 1)
        .take(top_n)
        .collect();

    if display.is_empty() {
        println!("  {}", "No multi-function modules to measure cohesion.".dimmed());
        return;
    }

    let max_label_len = display.iter().map(|c| c.module.len()).max().unwrap_or(5).min(25);

    println!(
        "\n  {:>width$}   {:<8} {:<8} {}",
        "Module".bold().underline(),
        "Call".bold().underline(),
        "Type".bold().underline(),
        "Combined".bold().underline(),
        width = max_label_len,
    );

    for mc in &display {
        let label = if mc.module.len() > max_label_len {
            format!("..{}", &mc.module[mc.module.len() - (max_label_len - 2)..])
        } else {
            mc.module.clone()
        };

        let combined_colored = if mc.combined < 0.1 {
            format!("{:.2}", mc.combined).red().bold().to_string()
        } else if mc.combined < 0.3 {
            format!("{:.2}", mc.combined).yellow().to_string()
        } else {
            format!("{:.2}", mc.combined).green().to_string()
        };

        println!(
            "  {:>width$}   {:<8} {:<8} {}  ({} fns)",
            label.dimmed(),
            format!("{:.2}", mc.call_cohesion),
            format!("{:.2}", mc.type_cohesion),
            combined_colored,
            mc.function_count,
            width = max_label_len,
        );
    }

    if cohesion.iter().filter(|c| c.function_count > 1).count() > display.len() {
        println!(
            "{}",
            format!(
                "  ... and {} more modules",
                cohesion.iter().filter(|c| c.function_count > 1).count() - display.len()
            )
            .dimmed()
        );
    }

    println!();
}

/// Render dependency analysis as JSON.
pub fn render_deps_json(rs_files: &[PathBuf], project_path: &Path) {
    let analysis = analyze_deps(rs_files, project_path);

    let edges: Vec<DepEdgeJson> = analysis
        .edges
        .iter()
        .map(|e| DepEdgeJson { from: e.from.clone(), to: e.to.clone() })
        .collect();

    let module_degrees: Vec<ModuleDegreeJson> = analysis
        .modules
        .iter()
        .map(|m| ModuleDegreeJson {
            module: m.clone(),
            in_degree: analysis.in_degree.get(m).copied().unwrap_or(0),
            out_degree: analysis.out_degree.get(m).copied().unwrap_or(0),
        })
        .collect();

    let coupling_pairs: Vec<CouplingPairJson> = analysis
        .coupling_pairs
        .iter()
        .map(|(a, b, s)| CouplingPairJson {
            module_a: a.clone(),
            module_b: b.clone(),
            strength: *s,
            direction: if *s >= 2 { "bidirectional".to_string() } else { "unidirectional".to_string() },
        })
        .collect();

    let cohesion = compute_cohesion(rs_files, project_path);

    let output = DepsJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        modules: analysis.modules,
        edges,
        module_degrees,
        coupling_pairs,
        cohesion,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}
