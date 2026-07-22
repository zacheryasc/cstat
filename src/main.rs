mod ast_parser;
mod call_trace;
mod circuits;
mod cluster;
mod complexity;
mod coverage;
mod datapaths;
mod dead_code;
mod deps;
mod diagnostics;
mod discovery;
mod dist;
mod dump;
mod file_metrics;
mod file_summary;
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

    /// Rust project directory or Rust source file to analyze (defaults to current directory)
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
    #[command(
        about = "Lines-of-code size-shape analysis",
        long_about = r#"Lines-of-code size-shape analysis for Rust source.

Project mode:
  cstat loc --path .
  cstat loc --path . --json

Selected-file mode:
  cstat loc --path src/lib.rs
  cstat loc --path src/lib.rs --json

Explain mode:
  cstat loc --explain
  cstat loc --explain --json

Project mode reports total physical lines, code_lines, aggregate stats, per-file ranking, and directory breakdown for discovered Rust files. Selected-file mode reports projected static reachable/unreachable production line spans for one Rust source file.

code_lines is a static textual classification: blank lines are excluded, // comment-only lines are excluded, and block-comment-only regions are excluded using simple /* ... */ state tracking. It is not semantic Rust parsing.

Project JSON fields: cstat_version, files[{path,total_lines,code_lines}], aggregate{total_files,total_loc,mean,std_dev,median,min,max}, directory_breakdown[{directory,code_lines}].
Selected-file JSON fields: file, total_lines, code_lines, projected_reachable_lines, projected_unreachable_lines, reachable_spans[{function,line_start,line_end}], unreachable_spans[{function,line_start,line_end}].

Use --explain to print this usage and JSON field contract without running analysis. --path, --top, and -v are ignored by --explain.

--top only limits the project-mode per-file ranking (ignored for selected-file and --explain modes)."#
    )]
    Loc {
        /// Show only the top N files in project mode (ignored for selected-file and --explain modes)
        #[arg(long)]
        top: Option<usize>,
        /// Print the loc usage and JSON field contract without running analysis
        #[arg(long)]
        explain: bool,
    },
    /// Rust AST symbol counts by kind, including trait impl blocks
    Symbols,
    /// Function decision/path complexity rankings
    Branching,
    /// Function API boundary complexity rankings
    Signature,
    /// Function implementation span rankings
    Span,
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
    /// Dynamic line and branch coverage from Rust source-based coverage data.
    Coverage {
        /// Parse an existing llvm-cov export JSON from CSTAT_LLVM_COV_EXPORT_JSON instead of running tests.
        #[arg(long)]
        no_run: bool,
    },
    /// Value-cluster transcript from AST def-use structure.
    Cluster,
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
    let command = cli.command.unwrap_or(Commands::Summary);

    if let Commands::Loc { explain: true, .. } = &command {
        loc::render_loc_explain(json);
        return;
    }
    let target = match discovery::resolve_target(&cli.path) {
        Ok(target) => target,
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
    let project_path = &target.project_path;
    let rs_files = discovery::files_for_project(&target);

    if let Some(file) = discovery::selected_file(&target) {
        let project_rs_files = discovery::files_for_project(&target);
        let target_rs_files = discovery::files_for_target(&target);
        match command {
            Commands::Summary | Commands::Report => {
                if json {
                    file_summary::render_file_summary_json(project_rs_files, project_path, file);
                } else {
                    file_summary::render_file_summary(
                        project_rs_files,
                        project_path,
                        file,
                        verbose,
                    );
                }
            }
            Commands::Loc { .. } => {
                if json {
                    loc::render_loc_file_json(project_rs_files, project_path, file);
                } else {
                    loc::render_loc_file(project_rs_files, project_path, file, verbose);
                }
            }
            Commands::Symbols => {
                if json {
                    symbols::render_symbols_file_json(project_rs_files, project_path, file);
                } else {
                    symbols::render_symbols_file(project_rs_files, project_path, file, verbose);
                }
            }
            Commands::Branching => {
                complexity::run_branching(target_rs_files, project_path, json, verbose);
            }
            Commands::Signature => {
                complexity::run_signature(target_rs_files, project_path, json, verbose);
            }
            Commands::Span => {
                complexity::run_span(target_rs_files, project_path, json, verbose);
            }
            Commands::Cluster => {
                if json {
                    cluster::render_cluster_file_json(project_rs_files, project_path, file);
                } else {
                    cluster::render_cluster_file(project_rs_files, project_path, file, verbose);
                }
            }
            Commands::Deps { .. } => {
                if json {
                    deps::render_deps_file_json(project_rs_files, project_path, file);
                } else {
                    deps::render_deps_file(project_rs_files, project_path, file, verbose);
                }
            }
            Commands::DeadCode => {
                if json {
                    dead_code::render_dead_code_file_json(project_rs_files, project_path, file);
                } else {
                    dead_code::render_dead_code_file(project_rs_files, project_path, file, verbose);
                }
            }
            Commands::TestReachability => {
                if json {
                    datapaths::render_test_reachability_file_json(
                        project_rs_files,
                        project_path,
                        file,
                    );
                } else {
                    datapaths::render_test_reachability_file(
                        project_rs_files,
                        project_path,
                        file,
                        verbose,
                    );
                }
            }
            Commands::CallTrace { entry } => {
                if let Err(message) = call_trace::run_file(
                    project_rs_files,
                    project_path,
                    file,
                    entry.as_deref(),
                    json,
                    verbose,
                ) {
                    if json {
                        let err = serde_json::json!({"error": message});
                        println!("{}", serde_json::to_string(&err).unwrap());
                    } else {
                        eprintln!("Error: {}", message);
                    }
                    std::process::exit(1);
                }
            }
            Commands::Coverage { no_run } => {
                if let Err(message) = coverage::render_coverage(
                    project_rs_files,
                    project_path,
                    Some(file),
                    no_run,
                    json,
                ) {
                    if json {
                        let err = serde_json::json!({"error": message});
                        println!("{}", serde_json::to_string(&err).unwrap());
                    } else {
                        eprintln!("Error: {}", message);
                    }
                    std::process::exit(1);
                }
            }
            Commands::Advanced { .. } => {
                let message =
                    "advanced commands require a project directory; pass --path <crate-dir>";
                if json {
                    let err = serde_json::json!({"error": message});
                    println!("{}", serde_json::to_string(&err).unwrap());
                } else {
                    eprintln!("Error: {}", message);
                }
                std::process::exit(1);
            }
        }
        return;
    }

    match command {
        Commands::Summary | Commands::Report => {
            if json {
                report::render_report_json(&rs_files, &project_path);
            } else {
                report::render_report(&rs_files, &project_path, verbose);
            }
        }
        Commands::Loc { top, .. } => {
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
        Commands::Branching => {
            complexity::run_branching(&rs_files, &project_path, json, verbose);
        }
        Commands::Signature => {
            complexity::run_signature(&rs_files, &project_path, json, verbose);
        }
        Commands::Span => {
            complexity::run_span(&rs_files, &project_path, json, verbose);
        }
        Commands::Cluster => {
            cluster::render_cluster(&rs_files, &project_path, json, verbose);
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
        Commands::Coverage { no_run } => {
            if let Err(message) =
                coverage::render_coverage(&rs_files, &project_path, None, no_run, json)
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
