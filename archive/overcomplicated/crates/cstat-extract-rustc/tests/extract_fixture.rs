//! Integration smoke test: spawn the compiled `cstat-extract-rustc`
//! binary against the §13 worked-example fixture, parse the resulting
//! JSON via `cstat_ir::CstatIrDump`, assert each IR layer is populated.
//!
//! This test is the executable form of Stage 12's done-when contract:
//! "running the binary on the fixture produces a valid cstat-ir JSON
//! dump covering HIR, THIR, MIR (pre and post opt), borrowck artifacts,
//! and mono-items".
//!
//! The test requires nightly's `librustc_driver*.so` on the dynamic
//! linker path; we derive `LD_LIBRARY_PATH` from `rustc --print
//! sysroot` so the test stays self-contained.

use std::path::PathBuf;
use std::process::Command;

use cstat_ir::CstatIrDump;

fn fixture_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates
    p.pop(); // workspace root
    p.push("examples");
    p.push("single-file-lib");
    p
}

fn rustc_sysroot() -> String {
    let out = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .expect("rustc --print sysroot");
    assert!(out.status.success(), "rustc --print sysroot failed");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn run_extractor(out_path: &std::path::Path) {
    let bin = env!("CARGO_BIN_EXE_cstat-extract-rustc");
    let sysroot = rustc_sysroot();
    let status = Command::new(bin)
        .arg("--path")
        .arg(fixture_path())
        .arg("--output")
        .arg(out_path)
        .env("LD_LIBRARY_PATH", format!("{sysroot}/lib"))
        .status()
        .expect("spawn cstat-extract-rustc");
    assert!(
        status.success(),
        "cstat-extract-rustc exited with {status:?}"
    );
}

fn extract_dump() -> CstatIrDump {
    let tmp = tempdir_path();
    std::fs::create_dir_all(&tmp).unwrap();
    let out = tmp.join("dump.json");
    run_extractor(&out);
    let txt = std::fs::read_to_string(&out).expect("read dump.json");
    serde_json::from_str(&txt).expect("parse CstatIrDump")
}

fn tempdir_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "cstat-extract-rustc-it-{}",
        std::process::id()
    ))
}

#[test]
fn fixture_dump_covers_every_ir_layer() {
    let dump = extract_dump();

    assert_eq!(dump.ir_format_version, cstat_ir::IR_FORMAT_VERSION);
    assert!(
        dump.toolchain.version.contains("rustc"),
        "toolchain version should look like rustc output: {:?}",
        dump.toolchain.version
    );

    // §13 contract: every required IR layer is present.
    let hir = dump.hir.as_ref().expect("hir section");
    let thir = dump.thir.as_ref().expect("thir section");
    let mir_preopt = dump.mir_preopt.as_ref().expect("mir_preopt section");
    let mir_opt = dump.mir_opt.as_ref().expect("mir_opt section");
    let borrowck = dump.borrowck.as_ref().expect("borrowck section");
    let mono = dump.mono_items.as_ref().expect("mono_items section");

    // Two pub fns in the fixture → both should appear as HIR items.
    let hir_fns: Vec<&str> = hir
        .items
        .iter()
        .filter(|i| i.kind == "fn")
        .map(|i| i.def_path.as_str())
        .collect();
    assert!(hir_fns.contains(&"add"), "HIR missing add: {hir_fns:?}");
    assert!(
        hir_fns.contains(&"classify"),
        "HIR missing classify: {hir_fns:?}"
    );

    // classify has two `if`s → branch_count ≥ 2.
    let classify_hir = hir
        .items
        .iter()
        .find(|i| i.def_path == "classify")
        .expect("classify in hir");
    assert!(
        classify_hir.branch_count >= 2,
        "classify branch_count {} should be ≥ 2 (two if exprs)",
        classify_hir.branch_count
    );

    // THIR / MIR / borrowck all see both bodies.
    for layer in [
        ("thir", thir.bodies.iter().map(|b| b.def_path.as_str()).collect::<Vec<_>>()),
        ("mir_preopt", mir_preopt.bodies.iter().map(|b| b.def_path.as_str()).collect()),
        ("mir_opt", mir_opt.bodies.iter().map(|b| b.def_path.as_str()).collect()),
        ("borrowck", borrowck.bodies.iter().map(|b| b.def_path.as_str()).collect()),
    ] {
        let (name, paths) = layer;
        assert!(paths.contains(&"add"), "{name} missing add: {paths:?}");
        assert!(
            paths.contains(&"classify"),
            "{name} missing classify: {paths:?}"
        );
    }

    // classify's MIR must have ≥ 4 basic blocks (entry + two if branches
    // + join). Exact count is rustc-version-sensitive so we lower-bound.
    let mir_classify = mir_opt
        .bodies
        .iter()
        .find(|b| b.def_path == "classify")
        .expect("classify in mir_opt");
    assert!(
        mir_classify.basic_blocks.len() >= 4,
        "classify basic_blocks {} should be ≥ 4",
        mir_classify.basic_blocks.len()
    );

    // Mono-items: both non-generic fns produce exactly one instantiation.
    let mono_paths: Vec<&str> = mono.items.iter().map(|m| m.def_path.as_str()).collect();
    assert!(
        mono_paths.contains(&"add"),
        "mono_items missing add: {mono_paths:?}"
    );
    assert!(
        mono_paths.contains(&"classify"),
        "mono_items missing classify: {mono_paths:?}"
    );
    for group in &mono.generic_instantiation_groups {
        assert_eq!(
            group.instantiation_count, 1,
            "non-generic {} should have exactly one instantiation",
            group.source_def_path
        );
    }
}

#[test]
fn fixture_dump_roundtrips_through_json_losslessly() {
    let dump = extract_dump();
    let json = serde_json::to_string(&dump).unwrap();
    let decoded: CstatIrDump = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, dump);
}
