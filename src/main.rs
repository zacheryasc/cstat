mod ast_parser;
mod circuits;
mod complexity;
mod datapaths;
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
mod summary;

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
    /// Full dashboard overview
    Summary,
    /// Lines-of-code analysis with bar charts
    Loc {
        /// Show only the top N files
        #[arg(long)]
        top: Option<usize>,
    },
    /// Per-function and per-file complexity rankings
    Complexity,
    /// Module dependency connectome
    Deps {
        /// Show only the top N most connected modules
        #[arg(long)]
        top: Option<usize>,
    },
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
    /// Execution flow heatmaps via random walk simulation
    Datapaths {
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

    match command {
        Commands::Summary => {
            let project_path = project_path.canonicalize().unwrap_or_else(|_| project_path.to_path_buf());
            if json {
                summary::render_summary_json(&rs_files, &project_path);
            } else {
                summary::render_summary(&rs_files, &project_path, verbose);
            }
        }
        Commands::Loc { top } => {
            let project_path = project_path.canonicalize().unwrap_or_else(|_| project_path.to_path_buf());
            if json {
                loc::render_loc_json(&rs_files, &project_path);
            } else {
                loc::render_loc(&rs_files, &project_path, top, verbose);
            }
        }
        Commands::Complexity => {
            let project_path = project_path.canonicalize().unwrap_or_else(|_| project_path.to_path_buf());
            let symbols = ast_parser::parse_project(&rs_files);
            if json {
                complexity::render_complexity_json(&symbols, &project_path);
            } else {
                complexity::render_complexity(&symbols, &project_path, verbose);
            }
        }
        Commands::Deps { top } => {
            let project_path = project_path.canonicalize().unwrap_or_else(|_| project_path.to_path_buf());
            if json {
                deps::render_deps_json(&rs_files, &project_path);
            } else {
                deps::render_deps(&rs_files, &project_path, top, verbose);
            }
        }
        Commands::Map => {
            let project_path = project_path.canonicalize().unwrap_or_else(|_| project_path.to_path_buf());
            if json {
                map::render_map_json(&rs_files, &project_path);
            } else {
                map::render_map(&rs_files, &project_path, verbose);
            }
        }
        Commands::Flow => {
            let project_path = project_path.canonicalize().unwrap_or_else(|_| project_path.to_path_buf());
            if json {
                flow::render_flow_json(&rs_files, &project_path);
            } else {
                flow::render_flow(&rs_files, &project_path, verbose);
            }
        }
        Commands::Graph { call_only, dep_only, top } => {
            let project_path = project_path.canonicalize().unwrap_or_else(|_| project_path.to_path_buf());
            if json {
                graph::render_graph_json(&rs_files, &project_path, call_only, dep_only);
            } else {
                graph::render_graph(&rs_files, &project_path, call_only, dep_only, top, verbose);
            }
        }
        Commands::Datapaths { walks, from } => {
            let project_path = project_path.canonicalize().unwrap_or_else(|_| project_path.to_path_buf());
            if json {
                datapaths::render_datapaths_json(&rs_files, &project_path, walks, from.as_deref());
            } else {
                datapaths::render_datapaths(&rs_files, &project_path, walks, from.as_deref(), verbose);
            }
        }
        Commands::Dist { metric, bins } => {
            let project_path = project_path.canonicalize().unwrap_or_else(|_| project_path.to_path_buf());
            if json {
                dist::render_dist_json(&rs_files, &project_path, metric.as_deref(), bins);
            } else {
                dist::render_dist(&rs_files, &project_path, metric.as_deref(), bins, verbose);
            }
        }
        Commands::Circuits { min_size } => {
            let project_path = project_path.canonicalize().unwrap_or_else(|_| project_path.to_path_buf());
            if json {
                circuits::render_circuits_json(&rs_files, &project_path, min_size);
            } else {
                circuits::render_circuits(&rs_files, &project_path, min_size, verbose);
            }
        }
        Commands::Redundancy => {
            let project_path = project_path.canonicalize().unwrap_or_else(|_| project_path.to_path_buf());
            if json {
                redundancy::render_redundancy_json(&rs_files, &project_path);
            } else {
                redundancy::render_redundancy(&rs_files, &project_path, verbose);
            }
        }
        Commands::Dump => {
            let project_path = project_path.canonicalize().unwrap_or_else(|_| project_path.to_path_buf());
            // dump is always JSON — --json flag is implicit
            colored::control::set_override(false);
            dump::render_dump(&rs_files, &project_path);
        }
        Commands::Guide { topic } => {
            match topic {
                Some(t) => guide::render_guide_topic(&t, json),
                None => guide::render_guide_toc(json),
            }
        }
    }
}
