mod ast_parser;
mod call_trace;
mod circuits;
mod complexity;
mod datapaths;
mod dead_code;
mod deps;
mod diagnostics;
mod discovery;
mod dist;
mod dump;
mod flow;
mod graph;
mod guide;
mod loc;
mod map;
mod redundancy;
mod render;
mod report;
mod summary;
mod symbols;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "cstat", about = "Rust codebase complexity analyzer")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Path to the Rust project directory (defaults to current directory)
    #[arg(long, default_value = ".", global = true)]
    path: PathBuf,

    /// Disable colored output
    #[arg(long, global = true)]
    no_color: bool,

    /// Print educational explanations for each section
    #[arg(short = 'v', long, global = true)]
    verbose: bool,

    /// Output results as structured JSON instead of human-readable format
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Focused module metrics report (default)
    Summary,
    /// Alias for the focused module metrics report
    Report,
    /// Lines-of-code analysis with bar charts
    Loc {
        /// Show only the top N files
        #[arg(long)]
        top: Option<usize>,
    },
    /// Symbol counts by kind and file
    Symbols,
    /// Per-function and per-file complexity rankings
    Complexity,
    /// Module dependency connectome
    Deps {
        /// Show only the top N most connected modules
        #[arg(long)]
        top: Option<usize>,
    },
    /// Static dead-code candidates
    DeadCode,
    /// Static test/benchmark reachability
    TestReachability,
    /// Tree view of calls from the Cargo binary entrypoint or an explicit function.
    CallTrace {
        /// Start from a specific function instead of the inferred cargo run entrypoint.
        #[arg(long, name = "PATH::FUNCTION")]
        entry: Option<String>,
    },
    /// Opt-in legacy and exploratory analyses
    Advanced {
        #[command(subcommand)]
        command: AdvancedCommands,
    },
}

#[derive(Subcommand)]
enum AdvancedCommands {
    /// Previous broad dashboard overview
    Summary,
    /// Bird's eye module map — architectural overview
    Map,
    /// Static call graph / execution flow analysis
    Flow,
    /// Graph-theoretic analysis (SCCs, centrality, PageRank, clustering)
    Graph {
        /// Analyze only the call graph
        #[arg(long)]
        call_only: bool,
        /// Analyze only the module dependency graph
        #[arg(long)]
        dep_only: bool,
        /// Show only the top N nodes per metric
        #[arg(long, default_value = "10")]
        top: usize,
    },
    /// Previous random-walk execution flow heatmap
    FlowHeatmap {
        /// Number of random walks to simulate
        #[arg(long, default_value = "10000")]
        walks: usize,
        /// Start from a specific function instead of all entry points
        #[arg(long, name = "FUNCTION")]
        from: Option<String>,
    },
    /// Distribution visualizations (histograms, scatter, outliers, correlations)
    Dist {
        /// Show only a single metric (loc, cyclomatic, cognitive, nesting, params)
        #[arg(long)]
        metric: Option<String>,
        /// Number of histogram bins
        #[arg(long, default_value = "10")]
        bins: usize,
    },
    /// Functional cluster decomposition via community detection
    Circuits {
        /// Minimum circuit size to display
        #[arg(long, default_value = "1")]
        min_size: usize,
    },
    /// Detect suspected duplicate or near-duplicate functions
    Redundancy,
    /// Machine-readable diagnostic summary (always JSON)
    Dump,
    /// Code health reference — topic-based workflow and metric guide
    Guide {
        /// Topic to display (e.g. getting-started, size, complexity, modularity, flow, redundancy, architecture)
        topic: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();

    let json = cli.json;

    if cli.no_color || json {
        colored::control::set_override(false);
    }

    let verbose = cli.verbose;
    let project_path = &cli.path;

    let rs_files = match discovery::discover_rs_files(project_path) {
        Ok(files) => files,
        Err(e) => {
            if json {
                let err = serde_json::json!({"error": format!("{}", e)});
                println!("{}", serde_json::to_string(&err).unwrap());
            } else {
                eprintln!("Error discovering files: {}", e);
            }
            std::process::exit(1);
        }
    };

    let command = cli.command.unwrap_or(Commands::Summary);

    let project_path = project_path
        .canonicalize()
        .unwrap_or_else(|_| project_path.to_path_buf());

    match command {
        Commands::Summary | Commands::Report => {
            if json {
                report::render_report_json(&rs_files, &project_path);
            } else {
                report::render_report(&rs_files, &project_path, verbose);
            }
        }
        Commands::Loc { top } => {
            if json {
                loc::render_loc_json(&rs_files, &project_path);
            } else {
                loc::render_loc(&rs_files, &project_path, top, verbose);
            }
        }
        Commands::Symbols => {
            if json {
                symbols::render_symbols_json(&rs_files, &project_path);
            } else {
                symbols::render_symbols(&rs_files, &project_path, verbose);
            }
        }
        Commands::Complexity => {
            complexity::run(&rs_files, &project_path, json, verbose);
        }
        Commands::Deps { top } => {
            if json {
                deps::render_deps_json(&rs_files, &project_path);
            } else {
                deps::render_deps(&rs_files, &project_path, top, verbose);
            }
        }
        Commands::DeadCode => {
            if json {
                dead_code::render_dead_code_json(&rs_files, &project_path);
            } else {
                dead_code::render_dead_code(&rs_files, &project_path, verbose);
            }
        }
        Commands::TestReachability => {
            if json {
                datapaths::render_test_reachability_json(&rs_files, &project_path);
            } else {
                datapaths::render_test_reachability(&rs_files, &project_path, verbose);
            }
        }
        Commands::CallTrace { entry } => {
            if let Err(message) =
                call_trace::run(&rs_files, &project_path, entry.as_deref(), json, verbose)
            {
                if json {
                    let err = serde_json::json!({"error": message});
                    println!("{}", serde_json::to_string(&err).unwrap());
                } else {
                    eprintln!("Error: {}", message);
                }
                std::process::exit(1);
            }
        }
        Commands::Advanced { command } => match command {
            AdvancedCommands::Summary => {
                if json {
                    summary::render_summary_json(&rs_files, &project_path);
                } else {
                    summary::render_summary(&rs_files, &project_path, verbose);
                }
            }
            AdvancedCommands::Map => {
                if json {
                    map::render_map_json(&rs_files, &project_path);
                } else {
                    map::render_map(&rs_files, &project_path, verbose);
                }
            }
            AdvancedCommands::Flow => {
                if json {
                    flow::render_flow_json(&rs_files, &project_path);
                } else {
                    flow::render_flow(&rs_files, &project_path, verbose);
                }
            }
            AdvancedCommands::Graph {
                call_only,
                dep_only,
                top,
            } => {
                if json {
                    graph::render_graph_json(&rs_files, &project_path, call_only, dep_only);
                } else {
                    graph::render_graph(
                        &rs_files,
                        &project_path,
                        call_only,
                        dep_only,
                        top,
                        verbose,
                    );
                }
            }
            AdvancedCommands::FlowHeatmap { walks, from } => {
                if json {
                    datapaths::render_datapaths_json(
                        &rs_files,
                        &project_path,
                        walks,
                        from.as_deref(),
                    );
                } else {
                    datapaths::render_datapaths(
                        &rs_files,
                        &project_path,
                        walks,
                        from.as_deref(),
                        verbose,
                    );
                }
            }
            AdvancedCommands::Dist { metric, bins } => {
                if json {
                    dist::render_dist_json(&rs_files, &project_path, metric.as_deref(), bins);
                } else {
                    dist::render_dist(&rs_files, &project_path, metric.as_deref(), bins, verbose);
                }
            }
            AdvancedCommands::Circuits { min_size } => {
                if json {
                    circuits::render_circuits_json(&rs_files, &project_path, min_size);
                } else {
                    circuits::render_circuits(&rs_files, &project_path, min_size, verbose);
                }
            }
            AdvancedCommands::Redundancy => {
                if json {
                    redundancy::render_redundancy_json(&rs_files, &project_path);
                } else {
                    redundancy::render_redundancy(&rs_files, &project_path, verbose);
                }
            }
            AdvancedCommands::Dump => {
                colored::control::set_override(false);
                dump::render_dump(&rs_files, &project_path);
            }
            AdvancedCommands::Guide { topic } => match topic {
                Some(t) => guide::render_guide_topic(&t, json),
                None => guide::render_guide_toc(json),
            },
        },
    }
}
