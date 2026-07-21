use colored::Colorize;
use serde::Serialize;

// ── Topic definitions ──────────────────────────────────────────────

pub const TOPICS: &[(&str, &str)] = &[
    (
        "getting-started",
        "First steps: reading your codebase with cstat",
    ),
    ("size", "File and function size, LoC distribution"),
    (
        "complexity",
        "Cyclomatic, cognitive, nesting, parameter metrics",
    ),
    (
        "modularity",
        "Coupling, cohesion, dependency structure, circuits",
    ),
    ("flow", "Call graph, reachability, dead code, hot paths"),
    ("redundancy", "Similar and duplicate function detection"),
    (
        "architecture",
        "Centrality, SCCs, hubs, tiers, graph topology",
    ),
];

struct TopicContent {
    description: &'static str,
    metrics: &'static [(&'static str, &'static str)],
    commands: &'static [&'static str],
    patterns: &'static [&'static str],
}

fn topic_getting_started() -> TopicContent {
    TopicContent {
        description: "\
cstat analyzes Rust codebases along several dimensions — size, complexity, \
dependency structure, call flow, and duplication — then surfaces observations \
about what it finds. Start with `cstat summary` for a human-readable dashboard, \
or `cstat dump` for a machine-readable JSON diagnostic report.",
        metrics: &[
            ("summary dashboard", "Compact overview of all dimensions with key stats"),
            ("dump report", "Full JSON diagnostic with health scores, observations, hotspots, and topology"),
            ("health scores", "Normalized 0.0–1.0 scores for modularity, complexity, and maintainability (higher = healthier)"),
        ],
        commands: &[
            "cstat summary           — human-readable dashboard (default command)",
            "cstat summary -v        — dashboard with educational explanations",
            "cstat dump              — machine-readable JSON diagnostic report",
            "cstat <command> --json  — structured JSON output for any command",
        ],
        patterns: &[
            "Human workflow: run summary first, read the dashboard, drill into specific \
dimensions with loc/complexity/deps/flow/graph as needed.",
            "Agent workflow: run dump for the full diagnostic picture, read scores to \
identify weak dimensions, use individual commands with --json to explore specifics.",
            "The -v (verbose) flag adds contextual explanations to any command — useful \
when learning what the metrics mean.",
        ],
    }
}

fn topic_size() -> TopicContent {
    TopicContent {
        description: "\
Size metrics measure the volume and distribution of Rust source. `cstat loc` owns the line-count contract; run `cstat loc --explain` for modes, code_lines rules, and JSON fields.",
        metrics: &[
            (
                "file size shape",
                "Project-level file-size distribution from `cstat loc --path .`.",
            ),
            (
                "selected-file line reachability",
                "Projected static reachable/unreachable production spans from `cstat loc --path <file.rs>`.",
            ),
            (
                "distribution statistics",
                "Mean, median, std_dev, min, and max summarize project file sizes.",
            ),
        ],
        commands: &[
            "cstat loc --explain       — loc modes, code_lines rules, and JSON field contract",
            "cstat loc --path .        — project size-shape report",
            "cstat loc --path src/lib.rs — selected-file projected line reachability",
            "cstat dist --metric loc — histogram and outlier analysis of LoC distribution",
        ],
        patterns: &[
            "Use `cstat loc --explain` as the canonical reference before consuming loc output.",
            "A right-skewed project-mode code_lines distribution means a few files dominate size.",
            "Selected-file loc is a static projection from function spans and test/benchmark reachability; do not read it as runtime coverage.",
        ],
    }
}

fn topic_complexity() -> TopicContent {
    TopicContent {
        description: "\
Complexity metrics quantify different aspects of how difficult code is to \
understand, test, and modify. No single metric captures the full picture — \
cyclomatic counts paths, cognitive models reading difficulty, nesting measures \
structural depth, and parameter count reflects interface width.",
        metrics: &[
            ("cyclomatic complexity", "Number of linearly independent paths through a function (branch_points + 1). Higher values mean more test cases needed for full coverage."),
            ("cognitive complexity", "SonarSource model: +1 per control flow break, +nesting_level penalty. Models how hard the code is for a human to read."),
            ("nesting depth", "Maximum depth of nested control structures. Deep nesting forces readers to hold more context in working memory."),
            ("parameter count", "Number of function parameters. Wide interfaces are harder to call correctly and may indicate a function doing too much."),
            ("composite score", "Weighted combination: 1.0×cyclomatic + 0.5×cognitive + 0.3×nesting + 0.2×generic + 0.1×param. Useful for ranking."),
        ],
        commands: &[
            "cstat complexity        — per-function and per-file complexity rankings",
            "cstat complexity --json — structured complexity data for all functions",
            "cstat dist --metric cyclomatic — distribution and outlier analysis",
            "cstat dist --metric cognitive  — cognitive complexity distribution",
        ],
        patterns: &[
            "High cyclomatic but low cognitive complexity usually means straightforward \
branching (e.g. match statements with simple arms). High cognitive with \
moderate cyclomatic often means nested conditionals.",
            "Functions with nesting depth > 4 are nearly always worth refactoring — \
extract inner blocks into helper functions to flatten the structure.",
            "The composite score is useful for triage: sort by composite to find functions \
that are complex along multiple dimensions simultaneously.",
        ],
    }
}

fn topic_modularity() -> TopicContent {
    TopicContent {
        description: "\
Modularity metrics describe the dependency structure between modules — how \
tightly they are coupled to each other and how cohesive each module is \
internally. Good modularity means changes tend to stay local rather than \
rippling across the codebase.",
        metrics: &[
            ("fan-in", "Number of modules that depend on this module. High fan-in modules are foundational — changes to them have wide impact."),
            ("fan-out", "Number of modules this module depends on. High fan-out may indicate a module that coordinates too many concerns."),
            ("bidirectional coupling", "Module pairs that depend on each other. Often a sign that the modules should be merged or the interface redesigned."),
            ("call cohesion", "Fraction of a module's functions that call at least one other function in the same module. Low values suggest the module groups unrelated functionality."),
            ("type cohesion", "Fraction of functions sharing parameter or return types with siblings. Measures data-level relatedness."),
            ("modularity Q", "Graph-theoretic partition quality from community detection. Higher values indicate well-separated clusters."),
        ],
        commands: &[
            "cstat deps              — module dependency graph with coupling and cohesion",
            "cstat deps --json       — structured dependency and cohesion data",
            "cstat circuits          — community detection showing functional clusters",
            "cstat circuits --json   — structured circuit membership data",
        ],
        patterns: &[
            "Bidirectional coupling pairs are the highest-priority modularity issue. \
They create change amplification — modifying either module risks breaking \
the other.",
            "A module with high fan-in AND high fan-out is a potential god module — it \
both serves many consumers and depends on many providers.",
            "Low cohesion (< 0.3) combined with large file size strongly suggests the \
module bundles unrelated concerns. Check if its functions form distinct \
clusters using `cstat circuits`.",
        ],
    }
}

fn topic_flow() -> TopicContent {
    TopicContent {
        description: "\
Flow analysis maps the call graph — which functions call which others — and \
uses it to identify entry points, dead code, hot paths, and structural \
properties of execution flow. This is static analysis; it shows what *can* \
be called, not runtime frequency.",
        metrics: &[
            ("entry points", "Functions reachable as starting points: main() and #[test] functions."),
            ("reachable functions", "Functions reachable from any entry point via the call graph."),
            ("cold functions", "Functions with zero incoming calls. May be dead code, or may be entry points for external consumers."),
            ("max call depth", "Longest chain of function calls. Very deep chains can indicate over-decomposition or recursion."),
            ("coverage", "Percentage of functions reachable from entry points. Low coverage means much of the code may be unused."),
            ("hot paths", "Paths through the call graph that visit the most functions. Shows the main execution spine."),
        ],
        commands: &[
            "cstat flow                     — call graph edges, entry points, cycles, cold functions",
            "cstat flow --json              — structured call graph data",
            "cstat test-reachability        — static test/benchmark reachability",
            "cstat test-reachability --json — structured test/benchmark reachability data",
            "cstat advanced flow-heatmap       — legacy random-walk heatmap",
            "cstat advanced flow-heatmap --json — structured visit counts and hot paths",
        ],
        patterns: &[
            "Cold functions that are not pub items or test helpers are likely dead code. \
Verify by checking if they appear in the module's public interface.",
            "Cycles in the call graph (recursive or mutually recursive functions) are \
worth noting — they make reasoning about termination harder and can cause \
stack overflows.",
            "Low coverage (< 70%) may indicate modules that are libraries consumed \
externally, or it may indicate accumulated dead code.",
        ],
    }
}

fn topic_redundancy() -> TopicContent {
    TopicContent {
        description: "\
Redundancy detection identifies functions that are structurally or \
signature-similar to each other. Duplicated logic is a maintenance burden — \
bug fixes need to be applied in multiple places, and divergent copies create \
subtle inconsistencies.",
        metrics: &[
            ("signature similarity", "Compares function name, parameter types, and return type. High similarity suggests functions that evolved from copy-paste."),
            ("structural similarity", "Normalizes function ASTs (strips identifiers, collapses literals) and compares hashes. Catches duplicates even when variable names differ."),
            ("similarity score", "Combined score from 0.0 to 1.0. Above 0.7 is suspicious; above 0.9 is near-certain duplication."),
            ("similarity kind", "Whether the match is by signature, structure, or both. 'Both' matches are the strongest signal."),
        ],
        commands: &[
            "cstat redundancy        — table of suspected duplicate function pairs",
            "cstat redundancy --json — structured similarity data for all pairs",
        ],
        patterns: &[
            "Structural duplicates with different names often indicate utility functions \
that were independently implemented in different modules. Consider extracting \
to a shared location.",
            "Signature-only matches (same parameter/return types, similar names) may be \
intentional polymorphism or may indicate an interface that should be a trait.",
            "A high count of redundant pairs in a single module suggests the module grew \
by accretion rather than design.",
        ],
    }
}

fn topic_architecture() -> TopicContent {
    TopicContent {
        description: "\
Architecture metrics apply graph theory to the call graph and dependency \
graph to reveal structural properties invisible at the function level — \
bridges, hubs, clusters, and overall connectivity patterns. These metrics \
describe the shape of the codebase.",
        metrics: &[
            ("betweenness centrality", "How often a node lies on shortest paths between other nodes. High-betweenness nodes are bridges — their removal disconnects the graph."),
            ("PageRank", "Recursive importance: a node is important if important nodes point to it. Identifies the most depended-upon functions."),
            ("SCCs (strongly connected components)", "Groups of nodes where every node can reach every other. In a call graph, these are mutual recursion groups. In a dependency graph, these are cyclic dependency clusters."),
            ("clustering coefficient", "How densely connected a node's neighbors are to each other. High values mean tight local clusters; low values mean the node bridges separate groups."),
            ("graph density", "Ratio of actual edges to possible edges. Very low density means a sparse, tree-like structure; higher density means more interconnection."),
        ],
        commands: &[
            "cstat graph             — SCC analysis, centrality, PageRank, clustering",
            "cstat graph --json      — structured graph metrics",
            "cstat graph --call-only — analyze only the call graph",
            "cstat graph --dep-only  — analyze only the module dependency graph",
            "cstat map               — architectural tier visualization",
        ],
        patterns: &[
            "High-betweenness nodes are refactoring leverage points: splitting them can \
decouple large portions of the codebase.",
            "Large SCCs in the dependency graph indicate tightly coupled module groups. \
These tend to grow over time as dependencies accumulate.",
            "A hub (high PageRank + high fan-out) that is also a bridge (high betweenness) \
is a critical risk point — it is both heavily depended upon and structurally \
load-bearing.",
        ],
    }
}

fn topic_content(name: &str) -> Option<TopicContent> {
    match name {
        "getting-started" => Some(topic_getting_started()),
        "size" => Some(topic_size()),
        "complexity" => Some(topic_complexity()),
        "modularity" => Some(topic_modularity()),
        "flow" => Some(topic_flow()),
        "redundancy" => Some(topic_redundancy()),
        "architecture" => Some(topic_architecture()),
        _ => None,
    }
}

// ── JSON serialization ─────────────────────────────────────────────

#[derive(Serialize)]
struct GuideTocJson {
    cstat_version: String,
    topics: Vec<GuideTopicEntry>,
}

#[derive(Serialize)]
struct GuideTopicEntry {
    name: String,
    summary: String,
}

#[derive(Serialize)]
struct GuideTopicJson {
    cstat_version: String,
    topic: String,
    description: String,
    metrics: Vec<GuideMetricJson>,
    commands: Vec<String>,
    patterns: Vec<String>,
}

#[derive(Serialize)]
struct GuideMetricJson {
    name: String,
    description: String,
}

// ── Public interface ───────────────────────────────────────────────

/// Render the guide table of contents (no topic specified).
pub fn render_guide_toc(json: bool) {
    if json {
        render_guide_toc_json();
    } else {
        render_guide_toc_human();
    }
}

/// Render a specific guide topic.
pub fn render_guide_topic(topic: &str, json: bool) {
    if json {
        render_guide_topic_json(topic);
    } else {
        render_guide_topic_human(topic);
    }
}

// ── Human-readable rendering ───────────────────────────────────────

fn render_guide_toc_human() {
    println!();
    println!(
        "{}",
        "cstat guide — code health reference".bright_cyan().bold()
    );
    println!();
    for (name, summary) in TOPICS {
        println!("  {:<18}{}", name.bold(), summary);
    }
    println!();
    println!("Run: {}", "cstat guide <topic>".white().bold());
    println!();
}

fn render_guide_topic_human(name: &str) {
    let content = match topic_content(name) {
        Some(c) => c,
        None => {
            eprintln!("Unknown guide topic: {}", name);
            eprintln!();
            eprintln!("Available topics:");
            for (t, _) in TOPICS {
                eprintln!("  {}", t);
            }
            std::process::exit(1);
        }
    };

    let topic_summary = TOPICS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| *s)
        .unwrap_or("");

    println!();
    println!(
        "{} — {}",
        format!("cstat guide {}", name).bright_cyan().bold(),
        topic_summary
    );

    // Description
    println!();
    for line in content.description.lines() {
        println!("  {}", line);
    }

    // Metrics
    if !content.metrics.is_empty() {
        println!();
        println!("{}", "  Key metrics:".bold());
        for (metric, desc) in content.metrics {
            println!();
            println!("    {}", metric.yellow().bold());
            // Wrap description text
            for line in desc.lines() {
                println!("      {}", line);
            }
        }
    }

    // Commands
    if !content.commands.is_empty() {
        println!();
        println!("{}", "  Relevant commands:".bold());
        for cmd in content.commands {
            println!("    {}", cmd.dimmed());
        }
    }

    // Patterns
    if !content.patterns.is_empty() {
        println!();
        println!("{}", "  Patterns worth noticing:".bold());
        for pattern in content.patterns {
            println!();
            print!("    • ");
            // Simple word-wrap at ~72 chars for readability
            let words: Vec<&str> = pattern.split_whitespace().collect();
            let mut col = 6; // indent of "    • "
            for (i, word) in words.iter().enumerate() {
                if i > 0 && col + 1 + word.len() > 76 {
                    println!();
                    print!("      ");
                    col = 6;
                } else if i > 0 {
                    print!(" ");
                    col += 1;
                }
                print!("{}", word);
                col += word.len();
            }
            println!();
        }
    }

    println!();
}

// ── JSON rendering ─────────────────────────────────────────────────

fn render_guide_toc_json() {
    let output = GuideTocJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        topics: TOPICS
            .iter()
            .map(|(name, summary)| GuideTopicEntry {
                name: name.to_string(),
                summary: summary.to_string(),
            })
            .collect(),
    };
    println!("{}", serde_json::to_string(&output).unwrap());
}

fn render_guide_topic_json(name: &str) {
    let content = match topic_content(name) {
        Some(c) => c,
        None => {
            let err = serde_json::json!({"error": format!("Unknown guide topic: {}", name)});
            println!("{}", serde_json::to_string(&err).unwrap());
            std::process::exit(1);
        }
    };

    let output = GuideTopicJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        topic: name.to_string(),
        description: content.description.to_string(),
        metrics: content
            .metrics
            .iter()
            .map(|(n, d)| GuideMetricJson {
                name: n.to_string(),
                description: d.to_string(),
            })
            .collect(),
        commands: content.commands.iter().map(|s| s.to_string()).collect(),
        patterns: content.patterns.iter().map(|s| s.to_string()).collect(),
    };
    println!("{}", serde_json::to_string(&output).unwrap());
}
