mod ast_parser;
mod call_trace;
mod cluster;
mod complexity;
mod coverage;
mod cpp;
mod datapaths;
mod deps;
mod discovery;
mod file_metrics;
mod flow;
mod loc;
mod render;
mod scorecard;
mod symbols;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Language {
    Rust,
    Cpp,
}

#[derive(Parser)]
#[command(name = "cstat", about = "Rust and C++ source complexity analyzer")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Rust project directory or Rust source file by default; C++ directory or source file with --language cpp
    #[arg(long, default_value = ".", global = true)]
    path: PathBuf,

    /// Source language (default Rust; select cpp for C++ source and headers)
    #[arg(long, value_enum, default_value = "rust", global = true)]
    language: Language,

    /// Exclude C++ paths matching a root-relative glob (repeatable)
    #[arg(long, value_name = "GLOB", global = true)]
    exclude: Vec<String>,

    /// C++ scorecard include lookup directory, absolute or relative to the analysis root (repeatable; no preprocessing)
    #[arg(long, value_name = "DIR", global = true)]
    include_dir: Vec<PathBuf>,

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

Project mode reports total_lines, code_lines, aggregate stats, per-file ranking, and directory breakdown for discovered Rust files. Selected-file mode reports projected static reachable/unreachable production line spans for one Rust source file.

code_lines is a static textual classification: blank lines are excluded, // comment-only lines are excluded, and block-comment-only regions are excluded using simple /* ... */ state tracking. It is not semantic Rust parsing.

Project-mode line counts are rustfmt-normalized before classification: every measured file is formatted with rustfmt (edition read from the nearest Cargo.toml) in a throwaway temp copy, then counted, so packing statements onto one line cannot deflate LOC. The working tree is never modified. Files rustfmt cannot parse, and any run where rustfmt is unavailable, fall back to raw textual counting. Selected-file mode is NOT rustfmt-normalized: its counts are real source lines whose spans tie to static reachability and coverage.

Project JSON fields: cstat_version, files[{path,total_lines,code_lines}], aggregate{total_files,total_loc,mean,std_dev,median,min,max}, directory_breakdown[{directory,code_lines}].
Selected-file JSON fields: file, total_lines, code_lines, projected_reachable_lines, projected_unreachable_lines, reachable_spans[{function,line_start,line_end}], unreachable_spans[{function,line_start,line_end}].

C++ mode (--language cpp) reports loc_mode: physical, totals, and per-file total_lines, code_lines, comment_lines, and blank_lines in both directory and selected-file modes. It does not format source or project static reachability.

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
    /// Rust AST symbol counts by kind, including trait impl blocks; C++ source symbols with --language cpp
    Symbols,
    /// Function decision/path complexity rankings
    Branching,
    /// Function API boundary complexity rankings
    Signature,
    /// Function implementation span rankings
    Span,
    #[command(
        about = "Deterministic structural code complexity scorecard",
        long_about = r#"Deterministic structural code complexity scorecard for Rust (default) or C++ (--language cpp).

C++ takes a source directory or a single source/header file; no build or manifest is needed.
It measures syntax as written, not a compiled configuration. Macros are not expanded, and
literal include lookup uses only the including file's directory (quoted includes) and
configured --include-dir roots. Use --exclude to omit root-relative paths.

C++ reports cpp_code_complexity_cost_v1. Compare only complete runs with equivalent selection
rules. Physical line counts are descriptive, not score inputs. --top limits contributors,
not the score or include graph. Incomplete analysis exits nonzero with no comparable score.
--changed [BASE] scores existing changed files plus nonignored untracked files, not affected
dependents. BASE defaults to HEAD. An empty C++ changed selection exits zero without a score.
With --diff-hunks, score only new-side changed C++ syntax, not whole files. This
reports cpp_diff_hunk_complexity_cost_v1, with new-side line ranges and contributors.
This mode does not resolve includes; omit --include-dir.

Example: cstat scorecard --language cpp --path ../project --include-dir include --json"#
    )]
    Scorecard {
        /// Show only the top N contributors
        #[arg(long)]
        top: Option<usize>,
        /// Score existing files of the selected language changed since BASE, including untracked files
        #[arg(long, value_name = "BASE", num_args = 0..=1, default_missing_value = "HEAD")]
        changed: Option<String>,
        /// Score only new-side changed syntax (requires --language cpp and --changed)
        #[arg(long, requires = "changed", conflicts_with = "include_dir")]
        diff_hunks: bool,
    },
    /// Module dependency connectome
    Deps {
        /// Show only the top N most connected modules
        #[arg(long)]
        top: Option<usize>,
    },
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

    if cli.language == Language::Cpp {
        if !run_cpp(&cli) {
            std::process::exit(1);
        }
        return;
    }
    if matches!(cli.command, Commands::Scorecard { diff_hunks: true, .. }) {
        discover_or_exit::<()>(Err("--diff-hunks requires --language cpp".into()), json);
    }
    if !cli.exclude.is_empty() || !cli.include_dir.is_empty() {
        discover_or_exit::<()>(
            Err("--exclude and --include-dir require --language cpp".into()),
            json,
        );
    }

    let verbose = cli.verbose;
    let command = cli.command;

    if let Commands::Loc { explain: true, .. } = &command {
        loc::render_loc_explain(json);
        return;
    }

    if let Commands::Scorecard {
        top,
        changed: Some(base),
        ..
    } = &command
    {
        let (project_path, files) =
            discover_or_exit(discovery::resolve_changed_target(&cli.path, base), json);
        scorecard::render_scorecard(
            &files,
            &project_path,
            &cli.path.display().to_string(),
            Some(base),
            *top,
            json,
            verbose,
        );
        return;
    }

    let target = discover_or_exit(discovery::resolve_target(&cli.path), json);
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
            Commands::Scorecard { top, .. } => {
                scorecard::render_scorecard(
                    target_rs_files,
                    project_path,
                    &cli.path.display().to_string(),
                    None,
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
        Commands::Scorecard { top, .. } => {
            scorecard::render_scorecard(
                &rs_files,
                &project_path,
                &cli.path.display().to_string(),
                None,
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

fn run_cpp(cli: &Cli) -> bool {
    let (report, base) = match &cli.command {
        Commands::Loc { explain: true, .. } => {
            cpp::explain_loc(cli.json);
            return true;
        }
        Commands::Loc { top, .. } => (cpp::Report::Loc { top: *top }, None),
        Commands::Symbols => (cpp::Report::Symbols, None),
        Commands::Branching => (cpp::Report::Branching, None),
        Commands::Signature => (cpp::Report::Signature, None),
        Commands::Span => (cpp::Report::Span, None),
        Commands::Scorecard { top, changed, diff_hunks } => {
            let report = if *diff_hunks {
                cpp::Report::DiffHunks { top: *top }
            } else {
                cpp::Report::Scorecard { top: *top }
            };
            (report, changed.as_deref())
        }
        Commands::Deps { .. } => return cpp_unsupported(cli.json, "deps"),
        Commands::TestReachability => return cpp_unsupported(cli.json, "test-reachability"),
        Commands::CallTrace { .. } => return cpp_unsupported(cli.json, "call-trace"),
        Commands::Coverage { .. } => return cpp_unsupported(cli.json, "coverage"),
        Commands::Cluster => return cpp_unsupported(cli.json, "cluster"),
    };

    let diff_hunks = matches!(report, cpp::Report::DiffHunks { .. });
    let selection = match discovery::cpp::select(&cli.path, base, &cli.exclude, &cli.include_dir, diff_hunks) {
        Ok(selection) => selection,
        Err(error) => return cpp_error(cli.json, &error),
    };
    cpp::run(&selection, report, cli.json, cli.verbose)
}

fn cpp_unsupported(json: bool, name: &str) -> bool {
    cpp_error(json, &format!("'{name}' is not supported with --language cpp"))
}

fn cpp_error(json: bool, message: &str) -> bool {
    if json {
        println!("{}", serde_json::json!({"language": "cpp", "status": "error", "error": message}));
    } else {
        eprintln!("Error: {message}");
    }
    false
}

fn discover_or_exit<T>(result: Result<T, String>, json: bool) -> T {
    match result {
        Ok(target) => target,
        Err(message) => {
            if json {
                println!("{}", serde_json::json!({"error": message}));
            } else {
                eprintln!("Error discovering files: {message}");
            }
            std::process::exit(1);
        }
    }
}
