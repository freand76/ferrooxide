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

impl Error {
    /// An error at `loc` (`file`: the preprocessed file where `loc` has none).
    pub fn at(msg: impl Into<String>, loc: syntax::tree::Loc, file: &str) -> Error {
        Error { msg: msg.into(), file: loc.file_or(file), line: loc.line }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: error: {}", loc_str(&self.file, self.line), self.msg)
    }
}

/// A part of a preprocessed file: from this line of the preprocessed text on, line `l` is
/// line `l + delta` of `file` (an included file, or the file itself between includes).
#[derive(Clone, Debug, PartialEq)]
pub struct LineSeg {
    pub start: u32,
    pub file: String,
    pub delta: i64,
}

/// The line maps of the preprocessed files, by file name. Locations in the AST are lines of
/// the preprocessed text; messages print them through `src_loc`.
static LINE_MAPS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Vec<LineSeg>>>> = std::sync::OnceLock::new();

pub(crate) fn set_line_map(file: &str, segs: Vec<LineSeg>) {
    let m = LINE_MAPS.get_or_init(Default::default);
    m.lock().unwrap().insert(file.to_string(), segs);
}

/// The source file and line of line `line` of the preprocessed `file`.
pub fn src_loc(file: &str, line: u32) -> (String, u32) {
    if let Some(m) = LINE_MAPS.get() {
        if let Some(segs) = m.lock().unwrap().get(file) {
            if let Some(s) = segs.iter().rev().find(|s| s.start <= line) {
                return (s.file.clone(), (line as i64 + s.delta).max(1) as u32);
            }
        }
    }
    (file.to_string(), line)
}

/// The names of the preprocessed files that `Loc::file` indexes (0: no file).
static FILES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The index of the preprocessed file `name` for `Loc::file` (from 1).
pub fn file_index(name: &str) -> u32 {
    let mut f = FILES.lock().unwrap();
    match f.iter().position(|n| n == name) {
        Some(i) => i as u32 + 1,
        None => {
            f.push(name.to_string());
            f.len() as u32
        }
    }
}

/// The name of the preprocessed file with index `i` (empty for 0).
pub fn file_name(i: u32) -> String {
    if i == 0 {
        return String::new();
    }
    FILES.lock().unwrap().get(i as usize - 1).cloned().unwrap_or_default()
}

/// `file:line` of line `line` of the preprocessed `file`, in the source.
pub fn loc_str(file: &str, line: u32) -> String {
    let (f, l) = src_loc(file, line);
    format!("{f}:{l}")
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
