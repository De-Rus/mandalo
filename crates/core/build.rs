use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The same extensions the browser build globs in `src/lib/web/seed.ts`. Both
/// hosts have to produce byte-identical trees, so the two lists are the one
/// thing that must not drift.
const EXTENSIONS: &[&str] = &[
    "toml", "http", "rest", "grpc", "ws", "mqtt", "proto", "json", "txt",
];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| EXTENSIONS.contains(&e))
        {
            out.push(path);
        }
    }
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/mock-workspace")
        .canonicalize()
        .expect("examples/mock-workspace must exist next to the crates");
    println!("cargo:rerun-if-changed={}", root.display());

    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "examples/mock-workspace holds no sample files"
    );

    let mut source = String::from("pub static SAMPLE_FILES: &[(&str, &str)] = &[\n");
    for path in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        let relative = path
            .strip_prefix(&root)
            .expect("walked below the root")
            .to_str()
            .expect("sample paths are utf-8")
            .replace('\\', "/");
        writeln!(
            source,
            "    ({:?}, include_str!({:?})),",
            relative,
            path.display().to_string()
        )
        .expect("writing to a String cannot fail");
    }
    source.push_str("];\n");

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"))
        .join("sample_files.rs");
    std::fs::write(&out, source).expect("cannot write the sample file table");
}
