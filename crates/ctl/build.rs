//! Compile an exact code-site map without embedding source text in the binary.
fn main() {
    let crate_dir = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = crate_dir.parent().unwrap().parent().unwrap();
    let list = crate_dir.join("analytics-sources.txt");
    println!("cargo:rerun-if-changed={}", list.display());
    let text = std::fs::read_to_string(list).expect("analytics source list");
    let mut hash = blake3::Hasher::new();
    let mut sources = Vec::new();
    for source in text.lines().filter(|line| !line.is_empty()) {
        assert!(source.starts_with("crates/") && source.ends_with(".rs") && !source.contains(".."));
        let path = root.join(source);
        println!("cargo:rerun-if-changed={}", path.display());
        hash.update(source.as_bytes()); hash.update(&[0]);
        hash.update(&std::fs::read(path).expect("analytics source file")); hash.update(&[0]);
        sources.push(source);
    }
    let output = format!("const SOURCES: &[&str] = &{sources:?};\nconst SOURCE_MANIFEST: &str = {:?};\n", hash.finalize().to_hex().to_string());
    std::fs::write(std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("analytics_sources.rs"), output).unwrap();
}
