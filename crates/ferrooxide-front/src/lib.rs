//! Verilog / SystemVerilog front end: preprocessor, lexer, AST and parser.

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod preproc;

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Error {
    pub msg: String,
    pub file: String,
    pub line: u32,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: error: {}", self.file, self.line, self.msg)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Default, Clone)]
pub struct Options {
    pub defines: HashMap<String, String>,
    pub incdirs: Vec<PathBuf>,
}

/// Parse a list of source files into modules.
pub fn parse_files(files: &[PathBuf], opts: &Options) -> Result<Vec<ast::Module>> {
    let mut pp = preproc::Preprocessor::new(opts);
    let mut modules = Vec::new();
    for f in files {
        let text = pp.process_file(f)?;
        let name = f.display().to_string();
        let toks = lexer::lex(&text, &name)?;
        let mut p = parser::Parser::new(toks, name);
        modules.extend(p.parse_source()?);
    }
    Ok(modules)
}

/// Parse source text directly (used by tests).
pub fn parse_str(text: &str, opts: &Options) -> Result<Vec<ast::Module>> {
    let mut pp = preproc::Preprocessor::new(opts);
    let text = pp.process_text(text, "<string>", std::path::Path::new("."))?;
    let toks = lexer::lex(&text, "<string>")?;
    parser::Parser::new(toks, "<string>".into()).parse_source()
}
