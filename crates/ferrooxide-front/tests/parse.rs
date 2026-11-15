//! Parser snapshot tests: every `tests/parse/*.v` / `*.sv` is parsed and printed with
//! `dump::modules`; the print must equal the checked-in `.ast` next to it. A directory
//! `tests/parse/<name>/` is one run over its files in name order (state carried from file to
//! file), compared with `<name>.ast`.
//! `FX_BLESS=1 cargo test -p ferrooxide-front --test parse` rewrites the `.ast` files.

use std::path::{Path, PathBuf};

#[test]
fn parse_snapshots() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/parse");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir() || is_source(p))
        .collect();
    files.sort();
    assert!(!files.is_empty());
    let bless = std::env::var_os("FX_BLESS").is_some();
    let mut failed = Vec::new();
    for f in &files {
        let opts = ferrooxide_front::Options::default();
        let srcs: Vec<PathBuf> = if f.is_dir() {
            let mut v: Vec<PathBuf> =
                std::fs::read_dir(f).unwrap().map(|e| e.unwrap().path()).filter(|p| is_source(p)).collect();
            v.sort();
            v
        } else {
            vec![f.clone()]
        };
        let got = match ferrooxide_front::parse_files(&srcs, &opts) {
            Ok(m) => ferrooxide_front::dump::modules(&m),
            Err(e) => format!("error: line {}: {}\n", e.line, e.msg),
        };
        let exp_path = if f.is_dir() { f.with_file_name(format!("{}.ast", f.file_name().unwrap().to_string_lossy())) } else { f.with_extension("ast") };
        if bless {
            std::fs::write(&exp_path, &got).unwrap();
            continue;
        }
        let exp = std::fs::read_to_string(&exp_path).unwrap_or_default();
        if got != exp {
            eprintln!("--- {} (expected)\n{exp}+++ (got)\n{got}", exp_path.display());
            failed.push(f.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    assert!(failed.is_empty(), "snapshot mismatch: {failed:?} (FX_BLESS=1 to update)");
}

fn is_source(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()), Some("v" | "sv"))
}

/// The new parser accepts every snapshot input and the parse-only files in `tests/syntax`.
#[test]
fn new_parser_accepts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut files: Vec<PathBuf> = Vec::new();
    for d in ["parse", "syntax"] {
        for e in std::fs::read_dir(root.join(d)).unwrap() {
            let p = e.unwrap().path();
            if is_source(&p) {
                files.push(p);
            } else if p.is_dir() {
                let mut v: Vec<PathBuf> = std::fs::read_dir(&p).unwrap().map(|e| e.unwrap().path()).filter(|p| is_source(p)).collect();
                v.sort();
                if let Err(e) = ferrooxide_front::syntax::parse_files(&v, &ferrooxide_front::Options::default()) {
                    panic!("{e}");
                }
            }
        }
    }
    files.sort();
    for f in &files {
        if let Err(e) = ferrooxide_front::syntax::parse_files(std::slice::from_ref(f), &ferrooxide_front::Options::default()) {
            panic!("{e}");
        }
    }
}
