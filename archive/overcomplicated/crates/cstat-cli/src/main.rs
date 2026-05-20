//! cstat CLI entry point.
//!
//! Single subcommand `extract` runs the representation orchestrator over a
//! Rust project and emits the spec §7 envelope as JSON.

use std::io::Write;
use std::path::PathBuf;

use anyhow::Context as _;
use clap::{Args, Parser, Subcommand};

use cstat_orchestrate::{Config, Context, Runner};

#[derive(Parser, Debug)]
#[command(
    name = "cstat",
    version,
    about = "Codebase-simplifier oracle: representation extraction"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Extract a representation envelope (spec §7) from a Rust project.
    Extract(ExtractArgs),
}

#[derive(Args, Debug)]
struct ExtractArgs {
    /// Path to the Rust project root (must contain Cargo.toml).
    #[arg(long)]
    path: PathBuf,

    /// Output file path. If omitted, JSON is written to stdout.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Enable the nightly rustc-internal extractor subprocess. Off by
    /// default; nightly-dependent levels will report `skipped` until the
    /// stable extractor pipeline is also complete.
    #[arg(long)]
    enable_rustc_internal: bool,

    /// Enable the spec §2.5 codegen levels (LLVM IR, assembly, object,
    /// binary). Off by default because these drive a real cargo build
    /// (cost class C); the five levels skip cleanly with a documented
    /// reason when this flag is absent.
    #[arg(long)]
    enable_codegen: bool,

    /// Override the toolchain channel stamped into the envelope
    /// (e.g. "stable", "nightly").
    #[arg(long)]
    toolchain: Option<String>,

    /// Emit compact (single-line) JSON. Default is multi-line pretty JSON.
    #[arg(long)]
    compact: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Extract(args) => run_extract(args),
    }
}

fn run_extract(args: ExtractArgs) -> anyhow::Result<()> {
    let project_root = args
        .path
        .canonicalize()
        .with_context(|| format!("project path {:?} could not be resolved", args.path))?;

    let ctx = Context {
        project_root,
        config: Config {
            enable_rustc_internal: args.enable_rustc_internal,
            enable_codegen: args.enable_codegen,
            toolchain_override: args.toolchain,
        },
    };

    let mut runner = Runner::new().with_cstat_version(env!("CARGO_PKG_VERSION"));
    cstat_extract_stable::register_all_stable(&mut runner);
    let envelope = runner.run(&ctx).context("extraction failed")?;

    let json = if args.compact {
        serde_json::to_string(&envelope)?
    } else {
        serde_json::to_string_pretty(&envelope)?
    };

    match args.output {
        Some(path) => {
            std::fs::write(&path, format!("{json}\n"))
                .with_context(|| format!("failed to write {:?}", path))?;
        }
        None => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            handle.write_all(json.as_bytes())?;
            handle.write_all(b"\n")?;
        }
    }
    Ok(())
}
