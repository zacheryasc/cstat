mod ast_parser;
mod call_trace;
mod cluster;
mod complexity;
mod coverage;
mod datapaths;
mod dead_code;
mod deps;
mod discovery;
mod file_metrics;
mod flow;
mod loc;
mod render;
mod scorecard;
mod symbols;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "cstat", about = "Rust codebase complexity analyzer")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

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
    /// Deterministic structural code complexity scorecard
    Scorecard {
        /// Show only the top N contributors
        #[arg(long)]
        top: Option<usize>,
    },
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
}

fn main() {
    let cli = Cli::parse();

    let json = cli.json;

    if cli.no_color || json {
        colored::control::set_override(false);
    }

    let verbose = cli.verbose;
    let command = cli.command;

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
            Commands::Scorecard { top } => {
                scorecard::render_scorecard(
                    target_rs_files,
                    project_path,
                    &cli.path.display().to_string(),
                    top,
                    json,
                    verbose,
                );
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
        }
        return;
    }

    match command {
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
        Commands::Scorecard { top } => {
            scorecard::render_scorecard(
                &rs_files,
                &project_path,
                &cli.path.display().to_string(),
                top,
                json,
                verbose,
            );
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
                coverage::render_coverage(&rs_files, &project_path, None, no_run, json, verbose)
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
    }
}
