//! The new front end: keywords, tokens, the syntax tree and its parser (IEEE 1800-2017).
//! `crate::lower` turns the tree into the AST the compiler reads.

pub mod kw;
pub mod parse;
pub mod token;
pub mod tree;

use crate::{preproc, Options, Result};
use std::path::PathBuf;

/// The name of the built-in `std` package's source (`STD_SOURCE`).
pub const STD_FILE: &str = "<std>";
const STD_SOURCE: &str = include_str!("../std.sv");

/// Preprocess and parse source files (one compilation unit unless `separate_units`). The
/// built-in package `std` comes first where the sources use its classes.
pub fn parse_files(files: &[PathBuf], opts: &Options) -> Result<Vec<tree::SourceFile>> {
    let mut pp = preproc::Preprocessor::new(opts);
    let mut texts = Vec::new();
    for (k, f) in files.iter().enumerate() {
        if opts.separate_units && k > 0 {
            pp = preproc::Preprocessor::new(opts);
        }
        let (text, segs) = pp.process_file_mapped(f)?;
        let name = f.display().to_string();
        crate::set_line_map(&name, segs);
        texts.push((text, name));
    }
    let mut state = parse::UnitState::default();
    let mut out = Vec::new();
    if texts.iter().any(|(t, _)| uses_std(t)) {
        let (sf, st) = parse::parse_file(STD_SOURCE, STD_FILE, state)?;
        state = st;
        out.push(sf);
    }
    for (k, (text, name)) in texts.iter().enumerate() {
        if opts.separate_units && k > 0 {
            state.unit_types.clear();
        }
        let (sf, st) = parse::parse_file(text, name, state)?;
        state = st;
        out.push(sf);
    }
    Ok(out)
}

/// The text names a class of package `std` (a word `mailbox`, `semaphore` or `process`).
fn uses_std(text: &str) -> bool {
    let b = text.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    ["mailbox", "semaphore", "process"].iter().any(|w| {
        text.match_indices(w).any(|(i, _)| (i == 0 || !word(b[i - 1])) && b.get(i + w.len()).is_none_or(|&c| !word(c)))
    })
}

/// Parse source text (tests).
pub fn parse_str(text: &str, opts: &Options) -> Result<Vec<tree::SourceFile>> {
    let mut pp = preproc::Preprocessor::new(opts);
    let text = pp.process_text(text, "<string>", std::path::Path::new("."))?;
    let (sf, _) = parse::parse_file(&text, "<string>", parse::UnitState::default())?;
    Ok(vec![sf])
}
