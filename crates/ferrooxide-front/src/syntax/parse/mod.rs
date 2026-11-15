//! Recursive-descent parser for IEEE 1800-2017 into the syntax tree (`super::tree`).
//!
//! The only semantic state is the set of type names in scope (typedefs, type parameters,
//! classes, covergroups, forward typedefs, and the type names packages export), needed to tell
//! `T x;` from an instance or a statement and to parse `T'(e)` and `$bits(T)`.

mod class;
mod expr;
mod items;
mod prop;
mod source;
mod stmt;
mod types;

use super::kw::Kw;
use super::token::{Token, T};
use super::tree::*;
use crate::{Error, Result};
use std::collections::{HashMap, HashSet};

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    file: String,
    /// type names per scope, innermost last; the first is the file scope (`$unit`)
    scopes: Vec<HashSet<String>>,
    /// type names each package declares (carried from file to file)
    pub packages: HashMap<String, HashSet<String>>,
    /// type names visible inside each class (its own and inherited ones)
    pub classes: HashMap<String, HashSet<String>>,
    /// in a constraint: `->` is an implication of constraint sets, not an operator
    in_constraint: bool,
}

/// What a parser carries from one file to the next of the same compilation unit.
#[derive(Default, Clone)]
pub struct UnitState {
    pub unit_types: HashSet<String>,
    pub packages: HashMap<String, HashSet<String>>,
    pub classes: HashMap<String, HashSet<String>>,
}

impl Parser {
    pub fn new(toks: Vec<Token>, file: String, state: UnitState) -> Parser {
        let mut packages = state.packages;
        // the built-in package `std` (process, semaphore, mailbox, randomize)
        packages.entry("std".into()).or_insert_with(|| ["process", "semaphore", "mailbox"].iter().map(|s| s.to_string()).collect());
        let mut unit = state.unit_types;
        let mut classes = state.classes;
        for t in ["process", "semaphore", "mailbox"] {
            unit.insert(t.to_string());
            classes.entry(t.to_string()).or_default();
        }
        Parser { toks, pos: 0, file, scopes: vec![unit], packages, classes, in_constraint: false }
    }

    pub fn into_state(mut self) -> UnitState {
        UnitState { unit_types: std::mem::take(&mut self.scopes[0]), packages: self.packages, classes: self.classes }
    }

    // ---------------------------------------------------------------- tokens

    pub(crate) fn peek(&self) -> &T {
        &self.toks[self.pos].t
    }
    pub(crate) fn peek_at(&self, k: usize) -> &T {
        let i = (self.pos + k).min(self.toks.len() - 1);
        &self.toks[i].t
    }
    pub(crate) fn line(&self) -> u32 {
        self.toks[self.pos].line
    }
    pub(crate) fn loc(&self) -> Loc {
        Loc { line: self.line() }
    }
    pub(crate) fn bump(&mut self) -> T {
        let t = self.toks[self.pos].t.clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    pub(crate) fn err<X>(&self, msg: impl Into<String>) -> Result<X> {
        Err(Error { msg: msg.into(), file: self.file.clone(), line: self.line() })
    }
    pub(crate) fn is_p(&self, p: &str) -> bool {
        matches!(self.peek(), T::Punct(q) if *q == p)
    }
    pub(crate) fn is_p_at(&self, k: usize, p: &str) -> bool {
        matches!(self.peek_at(k), T::Punct(q) if *q == p)
    }
    pub(crate) fn is_kw(&self, k: Kw) -> bool {
        matches!(self.peek(), T::Kw(q) if *q == k)
    }
    pub(crate) fn is_kw_at(&self, n: usize, k: Kw) -> bool {
        matches!(self.peek_at(n), T::Kw(q) if *q == k)
    }
    pub(crate) fn kw(&self) -> Option<Kw> {
        match self.peek() {
            T::Kw(k) => Some(*k),
            _ => None,
        }
    }
    pub(crate) fn eat_p(&mut self, p: &str) -> bool {
        if self.is_p(p) {
            self.bump();
            true
        } else {
            false
        }
    }
    pub(crate) fn eat_kw(&mut self, k: Kw) -> bool {
        if self.is_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }
    pub(crate) fn expect_p(&mut self, p: &str) -> Result<()> {
        if self.eat_p(p) {
            Ok(())
        } else {
            self.err(format!("expected '{p}', found {}", self.describe()))
        }
    }
    pub(crate) fn expect_kw(&mut self, k: Kw) -> Result<()> {
        if self.eat_kw(k) {
            Ok(())
        } else {
            self.err(format!("expected '{}', found {}", k.as_str(), self.describe()))
        }
    }
    pub(crate) fn describe(&self) -> String {
        describe(self.peek())
    }
    pub(crate) fn at_eof(&self) -> bool {
        matches!(self.peek(), T::Eof)
    }

    /// A name: an identifier, or a keyword IEEE 1800 added (legal as a name in Verilog code).
    pub(crate) fn ident(&mut self) -> Result<String> {
        match self.peek().clone() {
            T::Ident(s) => {
                self.bump();
                Ok(s)
            }
            T::Kw(k) if !k.is_hard() => {
                self.bump();
                Ok(k.as_str().to_string())
            }
            _ => self.err(format!("expected identifier, found {}", self.describe())),
        }
    }

    /// The token is a name (an identifier or a soft keyword).
    pub(crate) fn is_ident(&self) -> bool {
        self.is_ident_at(0)
    }
    pub(crate) fn is_ident_at(&self, k: usize) -> bool {
        match self.peek_at(k) {
            T::Ident(_) => true,
            T::Kw(k) => !k.is_hard(),
            _ => false,
        }
    }
    pub(crate) fn is_plain_ident_at(&self, k: usize) -> bool {
        matches!(self.peek_at(k), T::Ident(_))
    }

    /// Any name after `.` (members and methods may be spelled like keywords: `a.and()`).
    pub(crate) fn member_name(&mut self) -> Result<String> {
        match self.bump() {
            T::Ident(s) => Ok(s),
            T::Kw(k) => Ok(k.as_str().to_string()),
            _ => {
                self.pos -= 1;
                self.err(format!("expected a name after '.', found {}", self.describe()))
            }
        }
    }

    /// `: label` after `end`, `endmodule`, ...
    pub(crate) fn end_label(&mut self) -> Result<Option<String>> {
        if self.is_p(":") && !self.is_p("::") {
            self.bump();
            // `endfunction : new`
            if self.eat_kw(Kw::New) {
                return Ok(Some("new".into()));
            }
            return Ok(Some(self.ident()?));
        }
        Ok(None)
    }

    // ---------------------------------------------------------------- type names

    pub(crate) fn push_scope(&mut self) {
        self.scopes.push(HashSet::new());
    }
    pub(crate) fn pop_scope(&mut self) -> HashSet<String> {
        self.scopes.pop().unwrap_or_default()
    }
    pub(crate) fn add_type(&mut self, name: &str) {
        self.scopes.last_mut().unwrap().insert(name.to_string());
    }
    pub(crate) fn is_type_name(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(name))
    }
    /// `import p::*` / `import p::T`: the package's type names become visible.
    pub(crate) fn import_types(&mut self, pkg: &str, name: Option<&str>) {
        let Some(p) = self.packages.get(pkg) else { return };
        let names: Vec<String> = match name {
            None => p.iter().cloned().collect(),
            Some(n) => p.iter().filter(|t| *t == n).cloned().collect(),
        };
        for n in names {
            self.add_type(&n);
        }
    }

    /// Run `f` and go back to where it started if it fails (for the few places where the
    /// grammar needs to look further than a token or two).
    pub(crate) fn attempt<X>(&mut self, f: impl FnOnce(&mut Parser) -> Result<X>) -> Option<X> {
        let save = self.pos;
        let scopes = self.scopes.len();
        match f(self) {
            Ok(x) => Some(x),
            Err(_) => {
                self.pos = save;
                self.scopes.truncate(scopes.max(1));
                None
            }
        }
    }

    /// Skip a balanced `( ... )` / `[ ... ]` / `{ ... }` starting at the cursor.
    pub(crate) fn skip_balanced(&mut self) -> Result<()> {
        let mut depth = 0i32;
        loop {
            match self.bump() {
                T::Punct("(" | "[" | "{") => depth += 1,
                T::Punct(")" | "]" | "}") => {
                    depth -= 1;
                    if depth <= 0 {
                        return Ok(());
                    }
                }
                T::Eof => return self.err("unbalanced brackets"),
                _ => {}
            }
        }
    }
}

pub(crate) fn describe(t: &T) -> String {
    match t {
        T::Kw(k) => format!("'{}'", k.as_str()),
        T::Ident(s) | T::SysIdent(s) => format!("'{s}'"),
        T::Num(_) | T::Real(_) | T::Time(..) | T::Step => "number".into(),
        T::Str(_) => "string".into(),
        T::Punct(p) => format!("'{p}'"),
        T::Directive(_) => "compiler directive".into(),
        T::Eof => "end of file".into(),
    }
}

/// Parse one preprocessed file.
pub fn parse_file(text: &str, name: &str, state: UnitState) -> Result<(SourceFile, UnitState)> {
    let toks = super::token::lex(text, name)?;
    let mut p = Parser::new(toks, name.to_string(), state);
    let items = p.source_text()?;
    Ok((SourceFile { name: name.to_string(), items }, p.into_state()))
}
