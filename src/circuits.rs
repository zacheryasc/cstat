use colored::Colorize;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::flow;
use crate::render;

// ── Label Propagation Community Detection ────────────────────────

/// Detect communities in the call graph using label propagation.
/// Each function gets a label (initially its own index). Iteratively,
/// each node adopts the most frequent label among its neighbors.

/// Collect label frequency counts from a node's neighbors in both directions.
/// Returns a map of label -> occurrence count across all adjacent nodes.
fn collect_neighbor_labels(
    node: &str,
    adj: &HashMap<String, Vec<String>>,
    rev_adj: &HashMap<String, Vec<String>>,
    node_idx: &HashMap<&str, usize>,
    labels: &[usize],
) -> HashMap<usize, usize> {
    let mut label_counts: HashMap<usize, usize> = HashMap::new();

    if let Some(succs) = adj.get(node) {
        for s in succs {
            if let Some(&idx) = node_idx.get(s.as_str()) {
                *label_counts.entry(labels[idx]).or_insert(0) += 1;
            }
        }
    }
    if let Some(preds) = rev_adj.get(node) {
        for p in preds {
            if let Some(&idx) = node_idx.get(p.as_str()) {
                *label_counts.entry(labels[idx]).or_insert(0) += 1;
            }
        }
    }

    label_counts
}

/// Detect communities in the call graph using label propagation.
/// Each function gets a label (initially its own index). Iteratively,
/// each node adopts the most frequent label among its neighbors.
/// Converges when labels stabilize.
fn label_propagation(
    nodes: &[String],
    adj: &HashMap<String, Vec<String>>,
    rev_adj: &HashMap<String, Vec<String>>,
) -> Vec<Vec<String>> {
    let n = nodes.len();
    if n == 0 {
        return vec![];
    }

    let node_idx: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let mut labels: Vec<usize> = (0..n).collect();

    // Create a deterministic node ordering for iteration (sorted by name)
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| nodes[*a].cmp(&nodes[*b]));

    // Iterate until convergence or max iterations
    let max_iter = 50;
    for _ in 0..max_iter {
        let mut changed = false;

        for &i in &order {
            let label_counts = collect_neighbor_labels(&nodes[i], adj, rev_adj, &node_idx, &labels);

            if label_counts.is_empty() {
                continue;
            }

            // Pick the most frequent label; break ties by smallest label for determinism
            let max_count = *label_counts.values().max().unwrap();
            let best_label = label_counts
                .iter()
                .filter(|(_, &c)| c == max_count)
                .map(|(&l, _)| l)
                .min()
                .unwrap();

            if best_label != labels[i] {
                labels[i] = best_label;
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    // Group nodes by label
    let mut communities: HashMap<usize, Vec<String>> = HashMap::new();
    for (i, label) in labels.iter().enumerate() {
        communities
            .entry(*label)
            .or_default()
            .push(nodes[i].clone());
    }

    let mut result: Vec<Vec<String>> = communities.into_values().collect();
    // Sort communities by size (largest first), then by first member name for stability
    result.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].cmp(&b[0])));
    result
}

// ── Circuit Analysis ─────────────────────────────────────────────

struct Circuit {
    members: Vec<String>,
    boundary_fns: Vec<String>, // called from outside the circuit
    internal_edges: usize,
    cohesion: f64,
}

struct CircuitAnalysis {
    circuits: Vec<Circuit>,
    inter_circuit_edges: Vec<(usize, usize, usize)>, // (from_circuit, to_circuit, weight)
    modularity: f64,
}

/// Compute modularity Q for a given partition.
/// Q = (1/m) * Σ_ij [A_ij - (k_out_i * k_in_j / m)] * δ(c_i, c_j)
/// where m = total edges, k_out/k_in = degrees, δ = same community
fn compute_modularity(
    nodes: &[String],
    adj: &HashMap<String, Vec<String>>,
    partition: &HashMap<String, usize>, // node -> circuit index
) -> f64 {
    let m: usize = adj.values().map(|v| v.len()).sum();
    if m == 0 {
        return 0.0;
    }

    let mut out_deg: HashMap<&str, usize> = HashMap::new();
    let mut in_deg: HashMap<&str, usize> = HashMap::new();
    for n in nodes {
        out_deg.insert(n.as_str(), 0);
        in_deg.insert(n.as_str(), 0);
    }
    for (from, tos) in adj {
        *out_deg.entry(from.as_str()).or_insert(0) += tos.len();
        for to in tos {
            *in_deg.entry(to.as_str()).or_insert(0) += 1;
        }
    }

    let m_f = m as f64;
    let mut q = 0.0;

    for (from, tos) in adj {
        let c_from = match partition.get(from.as_str()) {
            Some(&c) => c,
            None => continue,
        };
        for to in tos {
            let c_to = match partition.get(to.as_str()) {
                Some(&c) => c,
                None => continue,
            };
            if c_from == c_to {
                let k_out = *out_deg.get(from.as_str()).unwrap_or(&0) as f64;
                let k_in = *in_deg.get(to.as_str()).unwrap_or(&0) as f64;
                q += 1.0 - (k_out * k_in / m_f);
            }
        }
    }

    q / m_f
}

/// Build Circuit structs from detected communities, computing boundary
/// functions, internal edges, and cohesion for each circuit.
fn build_circuits(
    communities: &[Vec<String>],
    cg: &flow::CallGraph,
    rev_adj: &HashMap<String, Vec<String>>,
    partition: &HashMap<String, usize>,
) -> Vec<Circuit> {
    let mut circuits: Vec<Circuit> = Vec::new();
    for (ci, community) in communities.iter().enumerate() {
        let member_set: HashSet<&str> = community.iter().map(|s| s.as_str()).collect();

        // Boundary functions: called from outside the circuit
        let mut boundary_fns = Vec::new();
        for member in community {
            let is_boundary = rev_adj.get(member.as_str()).map_or(false, |callers| {
                callers
                    .iter()
                    .any(|c| partition.get(c.as_str()).copied().unwrap_or(ci) != ci)
            });
            if is_boundary {
                boundary_fns.push(member.clone());
            }
        }
        boundary_fns.sort();

        // Internal edges: edges where both endpoints are in this circuit
        let mut internal_edges = 0usize;
        for member in community {
            if let Some(callees) = cg.adjacency.get(member.as_str()) {
                for callee in callees {
                    if member_set.contains(callee.as_str()) {
                        internal_edges += 1;
                    }
                }
            }
        }

        // Cohesion: internal edges / possible internal edges
        let n = community.len();
        let possible = if n > 1 { n * (n - 1) } else { 1 };
        let cohesion = internal_edges as f64 / possible as f64;

        circuits.push(Circuit {
            members: community.clone(),
            boundary_fns,
            internal_edges,
            cohesion,
        });
    }
    circuits
}

/// Compute inter-circuit edges: count directed call edges between each
/// pair of circuits, returned sorted by weight (heaviest first).
fn compute_inter_circuit_edges(
    cg: &flow::CallGraph,
    partition: &HashMap<String, usize>,
) -> Vec<(usize, usize, usize)> {
    let mut inter_edges: HashMap<(usize, usize), usize> = HashMap::new();
    for (caller, callees) in &cg.adjacency {
        let c_from = match partition.get(caller.as_str()) {
            Some(&c) => c,
            None => continue,
        };
        for callee in callees {
            let c_to = match partition.get(callee.as_str()) {
                Some(&c) => c,
                None => continue,
            };
            if c_from != c_to {
                *inter_edges.entry((c_from, c_to)).or_insert(0) += 1;
            }
        }
    }

    let mut result: Vec<(usize, usize, usize)> = inter_edges
        .into_iter()
        .map(|((a, b), w)| (a, b, w))
        .collect();
    result.sort_by(|a, b| b.2.cmp(&a.2));
    result
}

/// Run full circuit analysis on the call graph.
fn analyze_circuits(cg: &flow::CallGraph, min_size: usize) -> CircuitAnalysis {
    // Build reverse adjacency
    let mut rev_adj: HashMap<String, Vec<String>> = HashMap::new();
    for f in &cg.functions {
        rev_adj.entry(f.clone()).or_default();
    }
    for (caller, callees) in &cg.adjacency {
        for callee in callees {
            rev_adj
                .entry(callee.clone())
                .or_default()
                .push(caller.clone());
        }
    }

    let communities = label_propagation(&cg.functions, &cg.adjacency, &rev_adj);

    // Build partition map: node -> circuit index
    let mut partition: HashMap<String, usize> = HashMap::new();
    for (ci, community) in communities.iter().enumerate() {
        for node in community {
            partition.insert(node.clone(), ci);
        }
    }

    let circuits = build_circuits(&communities, cg, &rev_adj, &partition);
    let inter_circuit_edges = compute_inter_circuit_edges(cg, &partition);
    let modularity = compute_modularity(&cg.functions, &cg.adjacency, &partition);

    // Filter by min_size
    let filtered_circuits: Vec<Circuit> = circuits
        .into_iter()
        .filter(|c| c.members.len() >= min_size)
        .collect();

    CircuitAnalysis {
        circuits: filtered_circuits,
        inter_circuit_edges,
        modularity,
    }
}

fn render_overview(analysis: &CircuitAnalysis, total_functions: usize, verbose: bool) {
    println!(
        "\n{}",
        "── Circuit Overview ──────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Community detection via label propagation on the call graph.",
            "Each node starts with its own label; iteratively adopts the most frequent",
            "neighbor label until stable. Produces functional clusters (circuits).",
            "Modularity Q measures partition quality: Q > 0.3 = well-separated modules,",
            "  Q ≈ 0 = random partition, Q < 0 = worse than random.",
            "Sparkline shows relative circuit sizes (█ = largest, ▁ = smallest).",
        ]);
        render::guide_ref("modularity");
    }

    let sizes: Vec<usize> = analysis.circuits.iter().map(|c| c.members.len()).collect();

    println!();
    println!("  ┌──────────────────────────────────────┐");
    println!(
        "  │ {:<18} {:>17} │",
        "Circuits".cyan(),
        format!("{}", analysis.circuits.len()).bold()
    );
    println!(
        "  │ {:<18} {:>17} │",
        "Functions covered".cyan(),
        format!("{}", total_functions).bold()
    );
    if !sizes.is_empty() {
        let max_size = sizes.iter().max().unwrap_or(&0);
        let min_size = sizes.iter().min().unwrap_or(&0);
        let avg_size = sizes.iter().sum::<usize>() as f64 / sizes.len() as f64;
        println!(
            "  │ {:<18} {:>17} │",
            "Largest circuit".cyan(),
            format!("{}", max_size).bold()
        );
        println!(
            "  │ {:<18} {:>17} │",
            "Smallest circuit".cyan(),
            format!("{}", min_size).bold()
        );
        println!(
            "  │ {:<18} {:>17} │",
            "Average size".cyan(),
            format!("{:.1}", avg_size).bold()
        );
    }
    println!(
        "  │ {:<18} {:>17} │",
        "Modularity (Q)".cyan(),
        if analysis.modularity > 0.0 {
            format!("{:.4}", analysis.modularity)
                .green()
                .bold()
                .to_string()
        } else {
            format!("{:.4}", analysis.modularity)
                .red()
                .bold()
                .to_string()
        }
    );
    println!("  └──────────────────────────────────────┘");

    // Size distribution sparkline
    if sizes.len() > 1 {
        let max_s = *sizes.iter().max().unwrap_or(&1) as f64;
        let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let sparkline: String = sizes
            .iter()
            .map(|&s| {
                let idx = ((s as f64 / max_s) * 7.0).round() as usize;
                blocks[idx.min(7)]
            })
            .collect();
        println!(
            "\n  {} {}",
            "Size distribution:".dimmed(),
            sparkline.bright_yellow()
        );
    }
}

fn render_circuit_details(analysis: &CircuitAnalysis, verbose: bool) {
    println!(
        "\n{}",
        "── Circuits (by size) ────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Each circuit listed with its members, sorted by size (largest first).",
            "Cohesion = internal edges / possible internal edges. Higher = tighter cluster.",
            "  1.0 = every pair of members calls each other; 0.0 = no internal calls.",
            "◆ (blue) = boundary function: called from outside the circuit = the circuit's",
            "  API surface. These are the functions other circuits depend on.",
            "· (dim) = internal function: only called from within the same circuit.",
            "'Connects to' = other circuits this one exchanges calls with (weight = edge count).",
        ]);
    }

    for (i, circuit) in analysis.circuits.iter().enumerate() {
        println!();
        let header = format!(
            "  Circuit #{} — {} function{}",
            i + 1,
            circuit.members.len(),
            if circuit.members.len() == 1 { "" } else { "s" },
        );
        println!("{}", header.yellow().bold());

        // Stats line
        println!(
            "    Cohesion: {:.2}   Internal edges: {}   Boundary fns: {}",
            circuit.cohesion,
            circuit.internal_edges,
            circuit.boundary_fns.len(),
        );

        // Members, marking boundary functions
        let boundary_set: HashSet<&str> = circuit.boundary_fns.iter().map(|s| s.as_str()).collect();

        for member in &circuit.members {
            if boundary_set.contains(member.as_str()) {
                println!("    {} {}", "◆".bright_blue(), member.bright_white(),);
            } else {
                println!("    {} {}", "·".dimmed(), member.dimmed(),);
            }
        }

        // External dependencies for this circuit
        let circuit_deps: Vec<&(usize, usize, usize)> = analysis
            .inter_circuit_edges
            .iter()
            .filter(|(from, to, _)| *from == i || *to == i)
            .collect();

        if !circuit_deps.is_empty() {
            let mut dep_summary: Vec<(usize, usize)> = Vec::new(); // (other_circuit, total_weight)
            let mut seen: HashSet<usize> = HashSet::new();
            for &&(from, to, _w) in &circuit_deps {
                let other = if from == i { to } else { from };
                if seen.insert(other) {
                    // Sum both directions
                    let total: usize = analysis
                        .inter_circuit_edges
                        .iter()
                        .filter(|(a, b, _)| (*a == i && *b == other) || (*a == other && *b == i))
                        .map(|(_, _, w)| w)
                        .sum();
                    dep_summary.push((other, total));
                }
            }
            dep_summary.sort_by(|a, b| b.1.cmp(&a.1));

            print!("    {} ", "Connects to:".dimmed());
            let dep_strs: Vec<String> = dep_summary
                .iter()
                .take(5)
                .map(|(other, w)| format!("#{} ({})", other + 1, w))
                .collect();
            println!("{}", dep_strs.join(", ").dimmed());
        }
    }
}

fn render_inter_circuit_flow(analysis: &CircuitAnalysis, verbose: bool) {
    println!(
        "\n{}",
        "── Inter-Circuit Flow ────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Cross-circuit call volume between each pair of circuits.",
            "⇄ = bidirectional coupling (calls flow both ways between the two circuits).",
            "→ / ← = unidirectional flow. Number = total call edges between the pair.",
            "Bar length proportional to total edge count. Color: red = heaviest coupling,",
            "  yellow = moderate, green = light.",
            "High bidirectional coupling between circuits suggests they may belong together",
            "or need a cleaner interface boundary.",
        ]);
    }

    if analysis.inter_circuit_edges.is_empty() {
        println!("  {}", "No cross-circuit calls detected.".dimmed());
        return;
    }

    // Aggregate bidirectional: merge (a→b) and (b→a) into one row
    let mut pairs: HashMap<(usize, usize), (usize, usize)> = HashMap::new(); // (min, max) -> (fwd, bwd)
    for &(from, to, w) in &analysis.inter_circuit_edges {
        let key = if from <= to { (from, to) } else { (to, from) };
        let entry = pairs.entry(key).or_insert((0, 0));
        if from <= to {
            entry.0 += w;
        } else {
            entry.1 += w;
        }
    }

    let mut pair_list: Vec<((usize, usize), usize, usize)> = pairs
        .into_iter()
        .map(|((a, b), (fwd, bwd))| ((a, b), fwd, bwd))
        .collect();
    pair_list.sort_by(|a, b| (b.1 + b.2).cmp(&(a.1 + a.2)));

    let term_w = render::terminal_width();
    let bar_budget = 20.min(term_w.saturating_sub(40));
    let max_weight = pair_list.iter().map(|(_, f, b)| f + b).max().unwrap_or(1);

    println!();
    for ((a, b), fwd, bwd) in pair_list.iter().take(15) {
        let total = fwd + bwd;
        let bar_len = if max_weight > 0 {
            (total as f64 / max_weight as f64 * bar_budget as f64).ceil() as usize
        } else {
            0
        }
        .max(1);

        let bar = "█".repeat(bar_len);
        let ratio = total as f64 / max_weight as f64;
        let colored_bar = if ratio > 0.66 {
            bar.red().bold().to_string()
        } else if ratio > 0.33 {
            bar.yellow().to_string()
        } else {
            bar.green().to_string()
        };

        let direction = if *fwd > 0 && *bwd > 0 {
            format!("#{} ⇄ #{}", a + 1, b + 1)
        } else if *fwd > 0 {
            format!("#{} → #{}", a + 1, b + 1)
        } else {
            format!("#{} ← #{}", a + 1, b + 1)
        };

        println!(
            "    {:<16} {} {}",
            direction,
            colored_bar,
            format!("{} calls", total).dimmed(),
        );
    }

    if pair_list.len() > 15 {
        println!(
            "    {}",
            format!("... and {} more connections", pair_list.len() - 15).dimmed()
        );
    }

    println!();
}

/// Compact summary data for embedding in the dashboard.
pub struct CircuitSummary {
    /// Number of circuits.
    pub circuit_count: usize,
    /// Size of each circuit (largest first).
    pub sizes: Vec<usize>,
    /// Modularity score.
    pub modularity: f64,
}

/// Compute compact circuit summary for dashboard use.
pub fn circuit_summary(rs_files: &[PathBuf], project_path: &Path) -> Option<CircuitSummary> {
    let cg = flow::build_call_graph(rs_files, project_path);
    if cg.functions.is_empty() {
        return None;
    }
    let analysis = analyze_circuits(&cg, 1);
    if analysis.circuits.is_empty() {
        return None;
    }
    let sizes: Vec<usize> = analysis.circuits.iter().map(|c| c.members.len()).collect();
    Some(CircuitSummary {
        circuit_count: analysis.circuits.len(),
        sizes,
        modularity: analysis.modularity,
    })
}

// ── Public entry point ───────────────────────────────────────────

pub fn render_circuits(rs_files: &[PathBuf], project_path: &Path, min_size: usize, verbose: bool) {
    let cg = flow::build_call_graph(rs_files, project_path);

    if cg.functions.is_empty() {
        println!("{}", "No functions found for circuit analysis.".yellow());
        return;
    }

    let total_functions = cg.functions.len();
    let analysis = analyze_circuits(&cg, min_size);

    if analysis.circuits.is_empty() {
        println!(
            "{}",
            "No circuits found (try lowering --min-size).".yellow()
        );
        return;
    }

    render_overview(&analysis, total_functions, verbose);
    render_circuit_details(&analysis, verbose);
    render_inter_circuit_flow(&analysis, verbose);
}

#[derive(Serialize)]
struct CircuitJson {
    members: Vec<String>,
    boundary_functions: Vec<String>,
    internal_edges: usize,
    cohesion: f64,
}

#[derive(Serialize)]
struct InterCircuitEdgeJson {
    from_circuit: usize,
    to_circuit: usize,
    weight: usize,
}

#[derive(Serialize)]
struct CircuitsOutputJson {
    cstat_version: String,
    circuits: Vec<CircuitJson>,
    inter_circuit_edges: Vec<InterCircuitEdgeJson>,
    modularity: f64,
}

/// Render circuits analysis as JSON.
pub fn render_circuits_json(rs_files: &[PathBuf], project_path: &Path, min_size: usize) {
    let cg = flow::build_call_graph(rs_files, project_path);
    let analysis = analyze_circuits(&cg, min_size);

    let circuits: Vec<CircuitJson> = analysis
        .circuits
        .iter()
        .map(|c| CircuitJson {
            members: c.members.clone(),
            boundary_functions: c.boundary_fns.clone(),
            internal_edges: c.internal_edges,
            cohesion: c.cohesion,
        })
        .collect();

    let inter_circuit_edges: Vec<InterCircuitEdgeJson> = analysis
        .inter_circuit_edges
        .iter()
        .map(|(from, to, w)| InterCircuitEdgeJson {
            from_circuit: *from,
            to_circuit: *to,
            weight: *w,
        })
        .collect();

    let output = CircuitsOutputJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        circuits,
        inter_circuit_edges,
        modularity: analysis.modularity,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}
