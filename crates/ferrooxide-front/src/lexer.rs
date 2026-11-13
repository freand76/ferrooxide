use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct Number {
    pub size: Option<u32>,
    pub signed: bool,
    /// 2, 8, 10, 16. Unbased decimal literals have base 10 and `based == false`.
    pub base: u32,
    pub based: bool,
    /// lowercase digits, underscores removed ('?' mapped to 'z').
    pub digits: String,
    /// SV unbased-unsized fill literal: '0 '1 'x 'z
    pub fill: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    SysIdent(String),
    Num(Number),
    Real(f64),
    Str(String),
    Punct(&'static str),
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
}

const PUNCTS: &[&str] = &[
    "<<<=", ">>>=", "===", "!==", "<<<", ">>>", "<<=", ">>=", "==?", "!=?", "**", "==", "!=", "<=", ">=", "&&",
    "||", "<<", ">>", "~&", "~|", "~^", "^~", "+:", "-:", "->", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
    "++", "--", "::", "+", "-", "*", "/", "%", "<", ">", "=", "!", "&", "|", "^", "~", "?", ":", ";", ",", ".",
    "(", ")", "[", "]", "{", "}", "#", "@", "'",
];

pub fn lex(text: &str, fname: &str) -> Result<Vec<Token>> {
    let s = text.as_bytes();
    let mut i = 0;
    let mut line = 1u32;
    let mut toks = Vec::new();
    let err = |msg: String, line: u32| Error { msg, file: fname.to_string(), line };

    while i < s.len() {
        let c = s[i];
        if c == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // comments (normally removed by the preprocessor)
        if c == b'/' && i + 1 < s.len() && s[i + 1] == b'/' {
            while i < s.len() && s[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && i + 1 < s.len() && s[i + 1] == b'*' {
            i += 2;
            while i + 1 < s.len() && !(s[i] == b'*' && s[i + 1] == b'/') {
                if s[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 2;
            continue;
        }
        // attributes (* ... *) — but not @(*)
        if c == b'(' && i + 1 < s.len() && s[i + 1] == b'*' {
            let mut j = i + 2;
            while j < s.len() && s[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < s.len() && s[j] != b')' {
                i += 2;
                while i + 1 < s.len() && !(s[i] == b'*' && s[i + 1] == b')') {
                    if s[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
                i += 2;
                continue;
            }
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let st = i;
            while i < s.len() && (s[i].is_ascii_alphanumeric() || s[i] == b'_' || s[i] == b'$') {
                i += 1;
            }
            let word = &text[st..i];
            toks.push(Token { tok: Tok::Ident(word.to_string()), line });
            // a UDP table body does not tokenise like Verilog: keep it as raw text
            if word == "table" {
                if let Some(end) = text[i..].find("endtable") {
                    let raw = &text[i..i + end];
                    toks.push(Token { tok: Tok::Str(raw.to_string()), line });
                    line += raw.matches('\n').count() as u32;
                    i += end;
                }
            }
            continue;
        }
        if c == b'\\' {
            // escaped identifier
            i += 1;
            let st = i;
            while i < s.len() && !s[i].is_ascii_whitespace() {
                i += 1;
            }
            toks.push(Token { tok: Tok::Ident(text[st..i].to_string()), line });
            continue;
        }
        if c == b'$' && i + 1 < s.len() && (s[i + 1].is_ascii_alphabetic() || s[i + 1] == b'_') {
            let st = i;
            i += 1;
            while i < s.len() && (s[i].is_ascii_alphanumeric() || s[i] == b'_' || s[i] == b'$') {
                i += 1;
            }
            toks.push(Token { tok: Tok::SysIdent(text[st..i].to_string()), line });
            continue;
        }
        if c == b'"' {
            i += 1;
            let mut v = String::new();
            while i < s.len() && s[i] != b'"' {
                if s[i] == b'\\' && i + 1 < s.len() {
                    i += 1;
                    match s[i] {
                        b'n' => v.push('\n'),
                        b't' => v.push('\t'),
                        b'\\' => v.push('\\'),
                        b'"' => v.push('"'),
                        b'0'..=b'7' => {
                            let mut n = 0u32;
                            let mut k = 0;
                            while k < 3 && i < s.len() && (b'0'..=b'7').contains(&s[i]) {
                                n = n * 8 + (s[i] - b'0') as u32;
                                i += 1;
                                k += 1;
                            }
                            v.push(char::from_u32(n).unwrap_or('?'));
                            continue;
                        }
                        o => v.push(o as char),
                    }
                    i += 1;
                    continue;
                }
                if s[i] == b'\n' {
                    return Err(err("unterminated string".into(), line));
                }
                v.push(s[i] as char);
                i += 1;
            }
            i += 1;
            toks.push(Token { tok: Tok::Str(v), line });
            continue;
        }
        if c.is_ascii_digit() || (c == b'\'' && i + 1 < s.len() && is_base_start(&s[i + 1..])) {
            let (tok, ni) = lex_number(text, i).map_err(|m| err(m, line))?;
            i = ni;
            toks.push(Token { tok, line });
            continue;
        }
        // SV fill literals '0 '1 'x 'z
        if c == b'\'' && i + 1 < s.len() && matches!(s[i + 1], b'0' | b'1' | b'x' | b'X' | b'z' | b'Z') {
            let d = (s[i + 1] as char).to_ascii_lowercase();
            toks.push(Token {
                tok: Tok::Num(Number { size: None, signed: false, base: 2, based: true, digits: d.to_string(), fill: true }),
                line,
            });
            i += 2;
            continue;
        }
        let mut matched = false;
        for p in PUNCTS {
            if s[i..].starts_with(p.as_bytes()) {
                toks.push(Token { tok: Tok::Punct(p), line });
                i += p.len();
                matched = true;
                break;
            }
        }
        if !matched {
            return Err(err(format!("unexpected character '{}'", c as char), line));
        }
    }
    toks.push(Token { tok: Tok::Eof, line });
    Ok(toks)
}

fn is_base_start(s: &[u8]) -> bool {
    let mut k = 0;
    if k < s.len() && (s[k] == b's' || s[k] == b'S') {
        k += 1;
    }
    k < s.len() && matches!(s[k], b'b' | b'B' | b'o' | b'O' | b'd' | b'D' | b'h' | b'H')
}

fn lex_number(text: &str, mut i: usize) -> std::result::Result<(Tok, usize), String> {
    let s = text.as_bytes();
    let mut size = None;
    if s[i].is_ascii_digit() {
        let st = i;
        while i < s.len() && (s[i].is_ascii_digit() || s[i] == b'_') {
            i += 1;
        }
        let dec: String = text[st..i].chars().filter(|&c| c != '_').collect();
        // real?
        if i < s.len() && (s[i] == b'.' && i + 1 < s.len() && s[i + 1].is_ascii_digit() || s[i] == b'e' || s[i] == b'E')
        {
            let mut j = i;
            if s[j] == b'.' {
                j += 1;
                while j < s.len() && (s[j].is_ascii_digit() || s[j] == b'_') {
                    j += 1;
                }
            }
            if j < s.len() && (s[j] == b'e' || s[j] == b'E') {
                j += 1;
                if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
                    j += 1;
                }
                while j < s.len() && s[j].is_ascii_digit() {
                    j += 1;
                }
            }
            let t: String = text[st..j].chars().filter(|&c| c != '_').collect();
            let v: f64 = t.parse().map_err(|_| format!("bad real literal {t}"))?;
            return Ok((Tok::Real(v), j));
        }
        // look ahead for a base
        let mut j = i;
        while j < s.len() && (s[j] == b' ' || s[j] == b'\t') {
            j += 1;
        }
        if j < s.len() && s[j] == b'\'' && is_base_start(&s[j + 1..]) {
            size = Some(dec.parse::<u32>().map_err(|_| "bad size".to_string())?);
            i = j;
        } else {
            return Ok((
                Tok::Num(Number { size: None, signed: true, base: 10, based: false, digits: dec, fill: false }),
                i,
            ));
        }
    }
    // s[i] == '\''
    i += 1;
    let mut signed = false;
    if s[i] == b's' || s[i] == b'S' {
        signed = true;
        i += 1;
    }
    let base = match s[i].to_ascii_lowercase() {
        b'b' => 2,
        b'o' => 8,
        b'd' => 10,
        b'h' => 16,
        _ => return Err("bad base".into()),
    };
    i += 1;
    while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
        i += 1;
    }
    let st = i;
    while i < s.len() && (s[i].is_ascii_hexdigit() || matches!(s[i], b'_' | b'x' | b'X' | b'z' | b'Z' | b'?')) {
        i += 1;
    }
    let digits: String = text[st..i]
        .chars()
        .filter(|&c| c != '_')
        .map(|c| if c == '?' { 'z' } else { c.to_ascii_lowercase() })
        .collect();
    if digits.is_empty() {
        return Err("missing digits in based literal".into());
    }
    Ok((Tok::Num(Number { size, signed, base, based: true, digits, fill: false }), i))
}
