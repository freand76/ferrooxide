//! The new front end: keywords, tokens, the syntax tree and its parser (IEEE 1800-2017).
//! `crate::lower` turns the tree into the AST the compiler reads.

pub mod kw;
pub mod parse;
pub mod token;
pub mod tree;

use crate::{preproc, Options, Result};
use std::path::PathBuf;

/// Preprocess and parse source files (one compilation unit unless `separate_units`).
pub fn parse_files(files: &[PathBuf], opts: &Options) -> Result<Vec<tree::SourceFile>> {
    let mut pp = preproc::Preprocessor::new(opts);
    let mut state = parse::UnitState::default();
    let mut out = Vec::new();
    for (k, f) in files.iter().enumerate() {
        if opts.separate_units && k > 0 {
            pp = preproc::Preprocessor::new(opts);
            state.unit_types.clear();
        }
        let (text, segs) = pp.process_file_mapped(f)?;
        let name = f.display().to_string();
        crate::set_line_map(&name, segs);
        let (sf, st) = parse::parse_file(&text, &name, state)?;
        state = st;
        out.push(sf);
    }
    Ok(out)
}

/// Parse source text (tests).
pub fn parse_str(text: &str, opts: &Options) -> Result<Vec<tree::SourceFile>> {
    let mut pp = preproc::Preprocessor::new(opts);
    let text = pp.process_text(text, "<string>", std::path::Path::new("."))?;
    let (sf, _) = parse::parse_file(&text, "<string>", parse::UnitState::default())?;
    Ok(vec![sf])
}
