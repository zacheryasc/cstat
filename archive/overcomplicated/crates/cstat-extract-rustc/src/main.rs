//! Nightly `rustc_driver`-driven extractor sub-binary (Milestone 1 Stage 12).
//!
//! This binary is the **only** crate in the workspace that depends on
//! rustc-internal libraries. It is pinned to nightly via its own
//! `rust-toolchain.toml` and excluded from the workspace's default build
//! set so the stable side never pulls in `rustc_*`.
//!
//! Run shape (consumed by the stable orchestrator in Stage 13):
//!
//! ```text
//! cstat-extract-rustc --path <project-root> --output <out.json>
//! ```
//!
//! On success, writes a JSON dump matching [`cstat_ir::CstatIrDump`].
//! On nightly absence or rustc fatal error, exits non-zero and prints a
//! human-readable diagnostic to stderr — never a panic trace.

#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_target;

mod callbacks;
mod cli;
mod toolchain_stamp;
mod translate_borrowck;
mod translate_hir;
mod translate_mir;
mod translate_mono;
mod translate_thir;

use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::from(0),
        Err(exit) => exit,
    }
}

fn run() -> Result<(), ExitCode> {
    let args = cli::parse().map_err(|msg| {
        eprintln!("cstat-extract-rustc: {msg}");
        ExitCode::from(2)
    })?;

    let stamp = toolchain_stamp::probe();

    let sysroot = toolchain_stamp::sysroot().ok_or_else(|| {
        eprintln!("cstat-extract-rustc: failed to locate nightly sysroot");
        ExitCode::from(2)
    })?;

    let rustc_args = cli::build_rustc_args(&args, &sysroot).map_err(|e| {
        eprintln!("cstat-extract-rustc: {e}");
        ExitCode::from(2)
    })?;

    let mut cb = callbacks::ExtractCallbacks::new(stamp.clone());

    let driver_result = rustc_driver::catch_fatal_errors(|| {
        rustc_driver::run_compiler(&rustc_args, &mut cb);
    });

    if driver_result.is_err() {
        eprintln!("cstat-extract-rustc: rustc reported a fatal error during analysis");
        return Err(ExitCode::from(3));
    }

    let dump = cb.into_dump();
    let json = serde_json::to_string_pretty(&dump)
        .expect("cstat-ir dump is a closed shape and must serialize");
    std::fs::write(&args.output, json).map_err(|e| {
        eprintln!(
            "cstat-extract-rustc: failed to write {}: {e}",
            args.output.display()
        );
        ExitCode::from(4)
    })?;

    Ok(())
}
