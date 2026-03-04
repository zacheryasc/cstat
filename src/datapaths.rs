use colored::Colorize;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::path::PathBuf;

use crate::flow::{build_call_graph, CallGraph};
use crate::render;

/// Run many random walks through the call graph and accumulate visit counts.
fn simulate_walks(
    graph: &CallGraph,
    entry_points: &[String],
    num_walks: usize,
) -> HashMap<String, usize> {
    let mut visit_counts: HashMap<String, usize> = HashMap::new();
    let mut rng_state: u64 = 0xdeadbeef_u64;

    for (ep_idx, entry) in entry_points.iter().enumerate() {
        let walks_per_entry = num_walks / entry_points.len().max(1);
        for walk_i in 0..walks_per_entry {
            rng_state = rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add((ep_idx as u64).wrapping_mul(1442695040888963407).wrapping_add(walk_i as u64));
            let mut current = entry.clone();
            let mut steps = 0;
            let max_steps = 200;

            loop {
                *visit_counts.entry(current.clone()).or_insert(0) += 1;
                steps += 1;
                if steps >= max_steps {
                    break;
                }

                let callees = match graph.adjacency.get(&current) {
                    Some(c) if !c.is_empty() => c,
                    _ => break,
                };

                // Simple LCG random choice
                rng_state = rng_state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let idx = (rng_state >> 33) as usize % callees.len();
                current = callees[idx].clone();
            }
        }
    }

    visit_counts
}

/// Run walks from a single entry point.
fn simulate_walks_from(
    graph: &CallGraph,
    entry: &str,
    num_walks: usize,
) -> HashMap<String, usize> {
    let mut visit_counts: HashMap<String, usize> = HashMap::new();
    let mut rng_state: u64 = 0xcafebabe_u64;

    for walk_i in 0..num_walks {
        rng_state = rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(walk_i as u64);
        let mut current = entry.to_string();
        let mut steps = 0;
        let max_steps = 200;

        loop {
            *visit_counts.entry(current.clone()).or_insert(0) += 1;
            steps += 1;
            if steps >= max_steps {
                break;
            }

            let callees = match graph.adjacency.get(&current) {
                Some(c) if !c.is_empty() => c,
                _ => break,
            };

            rng_state = rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let idx = (rng_state >> 33) as usize % callees.len();
            current = callees[idx].clone();
        }
    }

    visit_counts
}

/// Find the hot path: greedily follow highest-visit-count callees from entry.
fn find_hot_path(graph: &CallGraph, entry: &str, visit_counts: &HashMap<String, usize>) -> Vec<String> {
    let mut path = vec![entry.to_string()];
    let mut visited = HashSet::new();
    visited.insert(entry.to_string());
    let mut current = entry.to_string();

    loop {
        let callees = match graph.adjacency.get(&current) {
            Some(c) if !c.is_empty() => c,
            _ => break,
        };

        // Pick callee with highest visit count
        let best = callees
            .iter()
            .filter(|c| !visited.contains(*c))
            .max_by_key(|c| visit_counts.get(*c).copied().unwrap_or(0));

        match best {
            Some(next) => {
                visited.insert(next.clone());
                path.push(next.clone());
                current = next.clone();
            }
            None => break,
        }
    }

    path
}

/// Find all functions reachable from any entry point via BFS.
fn reachable_from_entries(graph: &CallGraph) -> HashSet<String> {
    let mut reachable = HashSet::new();
    let mut queue: Vec<String> = graph.entry_points.clone();

    while let Some(node) = queue.pop() {
        if !reachable.insert(node.clone()) {
            continue;
        }
        if let Some(callees) = graph.adjacency.get(&node) {
            for callee in callees {
                if !reachable.contains(callee) {
                    queue.push(callee.clone());
                }
            }
        }
    }

    reachable
}

/// Render the heatmap ranked list.
fn render_heatmap(visit_counts: &HashMap<String, usize>, total_functions: usize, verbose: bool) {
    let tw = render::terminal_width();
    println!(
        "\n{}",
        "── Activation Heatmap ────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Random walk activation heatmap: N random walks (default 10,000) start from",
            "entry points and follow random callees at each step.",
            "Visit count ≈ runtime activation likelihood — how often each function would",
            "be hit if entry points were called uniformly at random.",
            "Color: red/bold = hot (>70% of max), yellow = warm (>40%), blue = cool (>15%),",
            "  dim = cold (<15%). Percentage = share of all visits across all walks.",
            "Coverage = fraction of total functions reached by at least one walk.",
        ]);
        render::guide_ref("flow");
    }

    let total_visits: usize = visit_counts.values().sum();
    if total_visits == 0 {
        println!("  {}", "No visits recorded.".dimmed());
        return;
    }

    let mut ranked: Vec<(&String, &usize)> = visit_counts.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1));

    let max_count = *ranked[0].1;
    let name_width = 40.min(tw.saturating_sub(30));
    let bar_max = 20.min(tw.saturating_sub(name_width + 20));

    println!();
    for (name, count) in ranked.iter().take(30) {
        let pct = **count as f64 / total_visits as f64 * 100.0;
        let bar_len = (**count as f64 / max_count as f64 * bar_max as f64) as usize;

        let heat = **count as f64 / max_count as f64;
        let bar = "█".repeat(bar_len);
        let colored_bar = if heat > 0.7 {
            bar.red().bold().to_string()
        } else if heat > 0.4 {
            bar.yellow().to_string()
        } else if heat > 0.15 {
            bar.bright_blue().to_string()
        } else {
            bar.dimmed().to_string()
        };

        let display_name = if name.len() > name_width {
            format!("…{}", &name[name.len() - name_width + 1..])
        } else {
            format!("{:<width$}", name, width = name_width)
        };

        println!(
            "  {} {} {:>5.1}%  {}",
            display_name.dimmed(),
            colored_bar,
            pct,
            format!("({})", count).dimmed(),
        );
    }

    if ranked.len() > 30 {
        println!(
            "  {} more functions with lower activation.",
            format!("… +{}", ranked.len() - 30).dimmed()
        );
    }

    println!(
        "\n  {} {} visits across {} functions",
        "Total:".bold(),
        total_visits,
        visit_counts.len(),
    );
    println!(
        "  {} {:.1}%",
        "Coverage:".bold(),
        visit_counts.len() as f64 / total_functions.max(1) as f64 * 100.0,
    );
}

/// Render per-entry-point breakdown.
fn render_per_entry(graph: &CallGraph, num_walks: usize, verbose: bool) {
    if graph.entry_points.is_empty() {
        return;
    }

    println!(
        "\n{}",
        "── Per-Entry-Point Breakdown ─────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Same random walks as above, but partitioned by starting entry point.",
            "Shows how many unique functions each entry point can reach and their",
            "top-5 most visited callees. Useful for understanding which entry point",
            "exercises which parts of the codebase.",
        ]);
    }

    for entry in &graph.entry_points {
        let counts = simulate_walks_from(graph, entry, num_walks / graph.entry_points.len().max(1));
        let total: usize = counts.values().sum();
        let mut ranked: Vec<(&String, &usize)> = counts.iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(a.1));

        println!("\n  {} (reached {} functions, {} total visits)",
            entry.bold().bright_white(),
            counts.len(),
            total,
        );

        let top5: Vec<_> = ranked.iter().take(5).collect();
        for (name, count) in &top5 {
            let pct = **count as f64 / total.max(1) as f64 * 100.0;
            println!("    {:<40} {:>5.1}%", name.dimmed(), pct);
        }
        if ranked.len() > 5 {
            println!("    {}", format!("… +{} more", ranked.len() - 5).dimmed());
        }
    }
}

/// Render hot path highlighting.
fn render_hot_paths(graph: &CallGraph, visit_counts: &HashMap<String, usize>, verbose: bool) {
    println!(
        "\n{}",
        "── Hot Paths ─────────────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Hot paths: greedily follow the highest-visit-count callee from each entry point.",
            "This traces the most likely execution path through the call graph.",
            "Arrow chain: entry → most visited callee → its most visited callee → ...",
            "Color: white = entry, red/bold = hot (>50% of max visits), yellow = warm (>20%),",
            "  plain = moderate, dim = cold. (depth N) = total chain length.",
        ]);
    }

    if graph.entry_points.is_empty() {
        println!("  {}", "No entry points found.".dimmed());
        return;
    }

    for entry in &graph.entry_points {
        let path = find_hot_path(graph, entry, visit_counts);
        if path.len() <= 1 {
            println!("\n  {} → {}", entry.bold().bright_white(), "(no callees)".dimmed());
            continue;
        }

        println!();
        print!("  ");
        for (i, node) in path.iter().enumerate() {
            let count = visit_counts.get(node).copied().unwrap_or(0);
            let colored_name = if i == 0 {
                node.bold().bright_white().to_string()
            } else if count > 0 {
                let max_count = visit_counts.values().max().copied().unwrap_or(1);
                let heat = count as f64 / max_count as f64;
                if heat > 0.5 {
                    node.red().bold().to_string()
                } else if heat > 0.2 {
                    node.yellow().to_string()
                } else {
                    node.to_string()
                }
            } else {
                node.dimmed().to_string()
            };

            if i > 0 {
                print!(" → ");
            }
            print!("{}", colored_name);
        }
        println!("  (depth {})", path.len());
    }
}

/// Render cold function detection.
fn render_cold_functions(graph: &CallGraph, visit_counts: &HashMap<String, usize>, verbose: bool) {
    let reachable = reachable_from_entries(graph);

    let cold: Vec<&String> = graph
        .functions
        .iter()
        .filter(|f| !visit_counts.contains_key(*f) && !reachable.contains(*f))
        .collect();

    println!(
        "\n{}",
        "── Cold Functions (Unreachable) ──────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Functions never reached from any entry point via call graph traversal.",
            "These are dead code candidates — they exist in the source but no entry point",
            "(main, #[test]) can reach them through any chain of calls.",
            "Note: static analysis may miss dynamic dispatch or macro-generated calls.",
        ]);
    }

    if cold.is_empty() {
        println!("  {}", "All functions are reachable from entry points.".green());
    } else {
        println!(
            "\n  {} functions never reached from any entry point (dead code candidates):\n",
            format!("{}", cold.len()).red().bold(),
        );
        for (i, name) in cold.iter().enumerate() {
            if i >= 20 {
                println!("  {}", format!("… +{} more", cold.len() - 20).dimmed());
                break;
            }
            println!("  {} {}", "•".red(), name.dimmed());
        }
    }

    println!();
}

/// Compact summary data for embedding in the dashboard.
pub struct DatapathSummary {
    /// Percentage of functions reachable from entry points.
    pub coverage_pct: f64,
    /// Total function count.
    pub total_functions: usize,
    /// Number of reachable functions.
    pub reachable_count: usize,
    /// Number of cold (unreachable) functions.
    pub cold_count: usize,
}

/// Compute compact datapath coverage for dashboard use.
pub fn datapath_summary(rs_files: &[PathBuf], project_path: &Path) -> Option<DatapathSummary> {
    let graph = build_call_graph(rs_files, project_path);
    if graph.functions.is_empty() {
        return None;
    }
    let total_functions = graph.functions.len();
    let reachable = reachable_from_entries(&graph);
    let reachable_count = reachable.len();
    let cold_count = total_functions.saturating_sub(reachable_count);
    let coverage_pct = reachable_count as f64 / total_functions.max(1) as f64 * 100.0;

    Some(DatapathSummary {
        coverage_pct,
        total_functions,
        reachable_count,
        cold_count,
    })
}

/// Public entry point: render the datapaths subcommand.
pub fn render_datapaths(
    rs_files: &[PathBuf],
    project_path: &Path,
    num_walks: usize,
    from_fn: Option<&str>,
    verbose: bool,
) {
    let graph = build_call_graph(rs_files, project_path);

    if graph.functions.is_empty() {
        println!("{}", "No functions found to analyze.".yellow());
        return;
    }

    let total_functions = graph.functions.len();

    // Determine entry points
    let entries: Vec<String> = if let Some(f) = from_fn {
        // Check if the function exists (exact or suffix match)
        let matched = graph.functions.iter().find(|name| {
            *name == f || name.ends_with(&format!("::{}", f))
        });
        match matched {
            Some(m) => vec![m.clone()],
            None => {
                eprintln!(
                    "{} Function '{}' not found in call graph.",
                    "Error:".red().bold(),
                    f
                );
                eprintln!("Available functions:");
                for func in graph.functions.iter().take(20) {
                    eprintln!("  {}", func);
                }
                if graph.functions.len() > 20 {
                    eprintln!("  … +{} more", graph.functions.len() - 20);
                }
                return;
            }
        }
    } else {
        if graph.entry_points.is_empty() {
            println!(
                "{} No entry points (main or #[test]) found. Use --from to specify a start function.",
                "Warning:".yellow().bold()
            );
            return;
        }
        graph.entry_points.clone()
    };

    println!(
        "\n{}",
        "══ Execution Flow Heatmap ═══════════════════════════════"
            .bright_cyan()
            .bold()
    );
    println!(
        "  {} random walks from {} entry point(s)",
        format!("{}", num_walks).bold(),
        entries.len(),
    );

    // Run global simulation
    let visit_counts = simulate_walks(&graph, &entries, num_walks);

    // Render sections
    render_heatmap(&visit_counts, total_functions, verbose);
    render_per_entry(&graph, num_walks, verbose);
    render_hot_paths(&graph, &visit_counts, verbose);
    render_cold_functions(&graph, &visit_counts, verbose);
}

#[derive(Serialize)]
struct VisitCountJson {
    function: String,
    visits: usize,
    percentage: f64,
}

#[derive(Serialize)]
struct DatapathsJson {
    cstat_version: String,
    visit_counts: Vec<VisitCountJson>,
    coverage_percentage: f64,
    hot_paths: Vec<Vec<String>>,
    cold_functions: Vec<String>,
    total_functions: usize,
    total_visits: usize,
}

/// Render datapaths analysis as JSON.
pub fn render_datapaths_json(
    rs_files: &[PathBuf],
    project_path: &Path,
    num_walks: usize,
    from_fn: Option<&str>,
) {
    let graph = build_call_graph(rs_files, project_path);
    let total_functions = graph.functions.len();

    let entries: Vec<String> = if let Some(f) = from_fn {
        let matched = graph.functions.iter().find(|name| {
            *name == f || name.ends_with(&format!("::{}", f))
        });
        match matched {
            Some(m) => vec![m.clone()],
            None => vec![],
        }
    } else {
        graph.entry_points.clone()
    };

    let visit_counts = simulate_walks(&graph, &entries, num_walks);
    let total_visits: usize = visit_counts.values().sum();

    let mut ranked: Vec<(&String, &usize)> = visit_counts.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1));

    let visit_counts_json: Vec<VisitCountJson> = ranked
        .iter()
        .map(|(name, count)| VisitCountJson {
            function: (*name).clone(),
            visits: **count,
            percentage: if total_visits > 0 { **count as f64 / total_visits as f64 * 100.0 } else { 0.0 },
        })
        .collect();

    let coverage = visit_counts.len() as f64 / total_functions.max(1) as f64 * 100.0;

    let hot_paths: Vec<Vec<String>> = entries
        .iter()
        .map(|entry| find_hot_path(&graph, entry, &visit_counts))
        .collect();

    let reachable = reachable_from_entries(&graph);
    let cold_functions: Vec<String> = graph
        .functions
        .iter()
        .filter(|f| !visit_counts.contains_key(*f) && !reachable.contains(*f))
        .cloned()
        .collect();

    let output = DatapathsJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        visit_counts: visit_counts_json,
        coverage_percentage: coverage,
        hot_paths,
        cold_functions,
        total_functions,
        total_visits,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}
