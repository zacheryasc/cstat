use std::{env, fs::File, io, path::PathBuf};

use flate2::read::GzDecoder;

fn main() -> io::Result<()> {
    for path in ["src/parser.c.gz", "src/scanner.c", "src/tree_sitter"] {
        println!("cargo:rerun-if-changed={path}");
    }

    let parser = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("parser.c");
    let mut tables = GzDecoder::new(File::open("src/parser.c.gz")?);
    io::copy(&mut tables, &mut File::create(&parser)?)?;

    let mut build = cc::Build::new();
    build.std("c11").include("src").files([parser, "src/scanner.c".into()]);
    if build.get_compiler().is_like_msvc() {
        build.flag("-utf-8");
    }
    build.compile("tree-sitter-cpp");
    Ok(())
}
