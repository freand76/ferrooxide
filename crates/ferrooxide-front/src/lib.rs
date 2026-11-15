//! Verilog / SystemVerilog front end: preprocessor, lexer, parser (`syntax`), the lowering
//! of its syntax tree (`lower`) and the AST the compiler reads.

pub mod ast;
pub mod dump;
pub mod lexer;
pub mod lower;
pub mod preproc;
pub mod syntax;

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
    /// every file is a compilation unit of its own (`-u`): macros, `timescale,
    /// `default_nettype and file-scope items do not carry over to the next file
    pub separate_units: bool,
}

/// Parse a list of source files into modules: the parser builds the syntax tree, the
/// lowering turns it into the AST the compiler reads.
pub fn parse_files(files: &[PathBuf], opts: &Options) -> Result<Vec<ast::Module>> {
    let tree = syntax::parse_files(files, opts)?;
    lower::lower_files(&tree, opts)
}

/// Parse source text directly (used by tests).
pub fn parse_str(text: &str, opts: &Options) -> Result<Vec<ast::Module>> {
    let tree = syntax::parse_str(text, opts)?;
    lower::lower_files(&tree, opts)
}
