//! Verilog / SystemVerilog front end: preprocessor, lexer, AST and parser.

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod preproc;
pub mod svtypes;

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

/// Parse a list of source files into modules.
pub fn parse_files(files: &[PathBuf], opts: &Options) -> Result<Vec<ast::Module>> {
    let mut pp = preproc::Preprocessor::new(opts);
    let mut modules = Vec::new();
    // `timescale stays in effect across files, in command-line order
    let mut timescale = None;
    let mut nettype = None;
    let mut uncdrive = None;
    // file-scope typedefs and items ($unit) stay visible in later files
    let mut types = HashMap::new();
    let mut unit = Vec::new();
    let mut unit_vars = Vec::new();
    let mut unit_var_types = HashMap::new();
    let mut unit_pkgs = Vec::new();
    let mut file_tu = (None, None);
    let mut packages = HashMap::new();
    let mut last: Option<parser::Parser> = None;
    for (k, f) in files.iter().enumerate() {
        if opts.separate_units && k > 0 {
            pp = preproc::Preprocessor::new(opts);
            timescale = None;
            nettype = None;
            uncdrive = None;
            types = HashMap::new();
            if let Some(mut p) = last.take() {
                p.unit_vars = std::mem::take(&mut unit_vars);
                p.unit_pkgs = std::mem::take(&mut unit_pkgs);
                p.unit_items = std::mem::take(&mut unit);
                modules.extend(p.unit_module());
            }
            unit_var_types = HashMap::new();
            file_tu = (None, None);
        }
        let text = pp.process_file(f)?;
        let name = f.display().to_string();
        let toks = lexer::lex(&text, &name)?;
        let mut p = parser::Parser::new(toks, name);
        p.timescale = timescale;
        p.default_nettype = nettype.clone();
        p.unconnected_drive = uncdrive;
        p.types = std::mem::take(&mut types);
        p.unit_items = std::mem::take(&mut unit);
        p.unit_vars = std::mem::take(&mut unit_vars);
        p.unit_var_types = std::mem::take(&mut unit_var_types);
        p.unit_pkgs = std::mem::take(&mut unit_pkgs);
        p.file_tu = file_tu;
        p.packages = std::mem::take(&mut packages);
        if opts.separate_units && k > 0 {
            p.unit_name = format!("$unit#{k}");
        }
        modules.extend(p.parse_source()?);
        types = std::mem::take(&mut p.types);
        unit = std::mem::take(&mut p.unit_items);
        unit_vars = std::mem::take(&mut p.unit_vars);
        unit_var_types = std::mem::take(&mut p.unit_var_types);
        unit_pkgs = std::mem::take(&mut p.unit_pkgs);
        file_tu = p.file_tu;
        packages = std::mem::take(&mut p.packages);
        timescale = p.timescale;
        nettype = p.default_nettype.clone();
        uncdrive = p.unconnected_drive;
        last = Some(p);
    }
    if let Some(mut p) = last {
        p.unit_vars = unit_vars;
        p.unit_pkgs = unit_pkgs;
        p.unit_items = unit;
        modules.extend(p.unit_module());
    }
    Ok(modules)
}

/// Parse source text directly (used by tests).
pub fn parse_str(text: &str, opts: &Options) -> Result<Vec<ast::Module>> {
    let mut pp = preproc::Preprocessor::new(opts);
    let text = pp.process_text(text, "<string>", std::path::Path::new("."))?;
    let toks = lexer::lex(&text, "<string>")?;
    let mut p = parser::Parser::new(toks, "<string>".into());
    let mut mods = p.parse_source()?;
    mods.extend(p.unit_module());
    Ok(mods)
}
