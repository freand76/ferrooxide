//! Tokens of the parser: the lexer's tokens with keywords classified and the
//! SystemVerilog operators the lexer splits joined again (only when the parts are
//! adjacent in the source, so `# #` stays two tokens).

use super::kw::Kw;
use crate::lexer::{self, Number, Tok};
use crate::Result;

#[derive(Clone, Debug, PartialEq)]
pub enum T {
    Kw(Kw),
    /// a name (escaped names without the backslash)
    Ident(String),
    SysIdent(String),
    Num(Number),
    Real(f64),
    /// a time literal: value and unit (power of ten of seconds)
    Time(f64, i32),
    /// `1step`
    Step,
    Str(String),
    Punct(&'static str),
    /// a directive from the preprocessor (`__fx_timescale`, `__fx_nettype`, `__fx_uncdrive`)
    Directive(String),
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub t: T,
    pub line: u32,
    pub pos: u32,
    pub end: u32,
}

/// Operators made of several old tokens: (parts, joined).
const JOINS: &[(&[&str], &str)] = &[
    (&["#", "-", "#"], "#-#"),
    (&["#", "=", "#"], "#=#"),
    (&["+", "*", ">"], "+*>"),
    (&["-", "*", ">"], "-*>"),
    (&["#", "#"], "##"),
    (&["|", "->"], "|->"),
    (&["|=", ">"], "|=>"),
    (&["<", "->"], "<->"),
    (&["->", ">"], "->>"),
    (&[":", "="], ":="),
    (&[":", "/"], ":/"),
    (&["@", "@"], "@@"),
    (&["+=", ">"], "+=>"),
    (&["-=", ">"], "-=>"),
    (&["=", ">"], "=>"),
    (&["*", ">"], "*>"),
];

/// Lex preprocessed text into the parser's tokens.
pub fn lex(text: &str, fname: &str) -> Result<Vec<Token>> {
    Ok(convert(lexer::lex(text, fname)?))
}

pub fn convert(toks: Vec<lexer::Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(toks.len());
    let mut i = 0;
    'next: while i < toks.len() {
        let tk = &toks[i];
        if let Tok::Punct(_) = tk.tok {
            for (parts, joined) in JOINS {
                if i + parts.len() <= toks.len()
                    && parts.iter().enumerate().all(|(k, p)| {
                        matches!(toks[i + k].tok, Tok::Punct(q) if q == *p) && (k == 0 || toks[i + k - 1].end == toks[i + k].pos)
                    })
                {
                    let last = &toks[i + parts.len() - 1];
                    out.push(Token { t: T::Punct(joined), line: tk.line, pos: tk.pos, end: last.end });
                    i += parts.len();
                    continue 'next;
                }
            }
        }
        // `1step`
        if let (Tok::Num(n), Some(next)) = (&tk.tok, toks.get(i + 1)) {
            if n.digits == "1" && !n.based && next.pos == tk.end && matches!(&next.tok, Tok::Ident(s) if s == "step") {
                out.push(Token { t: T::Step, line: tk.line, pos: tk.pos, end: next.end });
                i += 2;
                continue;
            }
        }
        let t = match &tk.tok {
            // an escaped name (`\module `) is never a keyword: the old lexer drops the backslash,
            // the source position tells
            Tok::Ident(s) if tk.end - tk.pos == s.len() as u32 => match Kw::from_str(s) {
                Some(k) => T::Kw(k),
                None if s.starts_with("__fx_") => T::Directive(s.clone()),
                None => T::Ident(s.clone()),
            },
            Tok::Ident(s) => T::Ident(s.clone()),
            Tok::SysIdent(s) => T::SysIdent(s.clone()),
            Tok::Num(n) => T::Num(n.clone()),
            Tok::Real(r) => T::Real(*r),
            Tok::Time(v, e) => T::Time(*v, *e),
            Tok::Str(s) => T::Str(s.clone()),
            Tok::Punct(p) => T::Punct(p),
            Tok::Eof => T::Eof,
        };
        out.push(Token { t, line: tk.line, pos: tk.pos, end: tk.end });
        i += 1;
    }
    out
}
