// Capture this crate's nightly sysroot at build time so the runtime
// driver doesn't have to guess. Cargo runs this against the
// rust-toolchain.toml-pinned toolchain, which is the same nightly that
// will dynamically link rustc_driver into the binary at run time. The
// pair is what keeps rustc_driver happy: querying with the
// wrong-versioned sysroot is the classic "VERSION_INCOMPATIBLE" trap.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=rust-toolchain.toml");

    let output = std::process::Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .expect("invoke rustc --print sysroot during build.rs");

    if !output.status.success() {
        panic!(
            "rustc --print sysroot failed during build: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let sysroot = String::from_utf8(output.stdout)
        .expect("rustc --print sysroot returned non-UTF-8")
        .trim()
        .to_string();

    println!("cargo:rustc-env=CSTAT_EXTRACT_RUSTC_SYSROOT={sysroot}");
}
