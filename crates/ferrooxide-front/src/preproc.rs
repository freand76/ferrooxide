//! A small Verilog preprocessor: `define (with arguments), `undef, `ifdef/`ifndef/`elsif/`else/`endif,
//! `include. Other directives (`timescale, `default_nettype, ...) are dropped.
//! Comments are stripped; line structure is preserved (directive lines become empty lines).

use crate::{Error, Options, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
struct Macro {
    params: Option<Vec<String>>,
    body: String,
}

pub struct Preprocessor {
    macros: HashMap<String, Macro>,
    incdirs: Vec<PathBuf>,
    depth: usize,
}

struct Cond {
    active: bool,
    taken: bool,
    parent_active: bool,
}

impl Preprocessor {
    pub fn new(opts: &Options) -> Self {
        let mut macros = HashMap::new();
        // Icarus-compatible output is the goal, so code written for Icarus takes its paths
        macros.insert("__ICARUS__".to_string(), Macro { params: None, body: "1".into() });
        for (k, v) in &opts.defines {
            macros.insert(k.clone(), Macro { params: None, body: v.clone() });
        }
        Preprocessor { macros, incdirs: opts.incdirs.clone(), depth: 0 }
    }

    pub fn process_file(&mut self, path: &Path) -> Result<String> {
        let text = std::fs::read_to_string(path).map_err(|e| Error {
            msg: format!("cannot read file: {e}"),
            file: path.display().to_string(),
            line: 0,
        })?;
        let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        self.process_text(&text, &path.display().to_string(), &dir)
    }

    pub fn process_text(&mut self, text: &str, fname: &str, dir: &Path) -> Result<String> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(Error { msg: "include/macro nesting too deep".into(), file: fname.into(), line: 0 });
        }
        let r = self.run(text, fname, dir);
        self.depth -= 1;
        r
    }

    fn run(&mut self, text: &str, fname: &str, dir: &Path) -> Result<String> {
        let s: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        let mut line = 1u32;
        let mut conds: Vec<Cond> = Vec::new();
        let err = |msg: String, line: u32| Error { msg, file: fname.to_string(), line };
        let active = |conds: &Vec<Cond>| conds.last().map(|c| c.active).unwrap_or(true);

        while i < s.len() {
            let c = s[i];
            // comments
            if c == '/' && i + 1 < s.len() && s[i + 1] == '/' {
                while i < s.len() && s[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            if c == '/' && i + 1 < s.len() && s[i + 1] == '*' {
                i += 2;
                while i < s.len() && !(s[i] == '*' && i + 1 < s.len() && s[i + 1] == '/') {
                    if s[i] == '\n' {
                        line += 1;
                        out.push('\n');
                    }
                    i += 1;
                }
                i += 2;
                out.push(' ');
                continue;
            }
            if c == '\n' {
                line += 1;
                out.push('\n');
                i += 1;
                continue;
            }
            if c == '"' {
                // string literal
                let start = i;
                i += 1;
                while i < s.len() && s[i] != '"' && s[i] != '\n' {
                    if s[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
                if active(&conds) {
                    out.extend(&s[start..i.min(s.len())]);
                }
                continue;
            }
            if c == '`' {
                i += 1;
                let st = i;
                while i < s.len() && (s[i].is_alphanumeric() || s[i] == '_' || s[i] == '$') {
                    i += 1;
                }
                let name: String = s[st..i].iter().collect();
                match name.as_str() {
                    "ifdef" | "ifndef" | "elsif" => {
                        let id = read_ident(&s, &mut i);
                        let defined = self.macros.contains_key(&id);
                        match name.as_str() {
                            "ifdef" | "ifndef" => {
                                let pa = active(&conds);
                                let cond = if name == "ifdef" { defined } else { !defined };
                                conds.push(Cond { active: pa && cond, taken: cond, parent_active: pa });
                            }
                            _ => {
                                let c = conds.last_mut().ok_or_else(|| err("`elsif without `ifdef".into(), line))?;
                                if c.taken {
                                    c.active = false;
                                } else {
                                    c.active = c.parent_active && defined;
                                    c.taken = defined;
                                }
                            }
                        }
                    }
                    "else" => {
                        let c = conds.last_mut().ok_or_else(|| err("`else without `ifdef".into(), line))?;
                        c.active = c.parent_active && !c.taken;
                        c.taken = true;
                    }
                    "endif" => {
                        conds.pop().ok_or_else(|| err("`endif without `ifdef".into(), line))?;
                    }
                    _ if !active(&conds) => {}
                    "define" => {
                        let id = read_ident(&s, &mut i);
                        let mut params = None;
                        if i < s.len() && s[i] == '(' {
                            i += 1;
                            let mut ps = Vec::new();
                            let mut cur = String::new();
                            while i < s.len() && s[i] != ')' {
                                if s[i] == ',' {
                                    ps.push(cur.trim().to_string());
                                    cur.clear();
                                } else {
                                    cur.push(s[i]);
                                }
                                i += 1;
                            }
                            i += 1;
                            if !cur.trim().is_empty() {
                                ps.push(cur.trim().to_string());
                            }
                            params = Some(ps);
                        }
                        // body until end of line, honoring backslash continuation
                        let mut body = String::new();
                        while i < s.len() && s[i] != '\n' {
                            if s[i] == '\\' && i + 1 < s.len() && s[i + 1] == '\n' {
                                body.push(' ');
                                out.push('\n');
                                line += 1;
                                i += 2;
                                continue;
                            }
                            if s[i] == '/' && i + 1 < s.len() && s[i + 1] == '/' {
                                while i < s.len() && s[i] != '\n' {
                                    i += 1;
                                }
                                break;
                            }
                            body.push(s[i]);
                            i += 1;
                        }
                        self.macros.insert(id, Macro { params, body: body.trim().to_string() });
                    }
                    "undef" => {
                        let id = read_ident(&s, &mut i);
                        self.macros.remove(&id);
                    }
                    "include" => {
                        while i < s.len() && s[i] == ' ' {
                            i += 1;
                        }
                        let close = if i < s.len() && s[i] == '<' { '>' } else { '"' };
                        i += 1;
                        let st = i;
                        while i < s.len() && s[i] != close && s[i] != '\n' {
                            i += 1;
                        }
                        let inc: String = s[st..i].iter().collect();
                        i += 1;
                        let mut found = None;
                        let cwd = std::path::PathBuf::from(".");
                        for d in std::iter::once(dir.to_path_buf()).chain(self.incdirs.iter().cloned()).chain(std::iter::once(cwd)) {
                            let p = d.join(&inc);
                            if p.exists() {
                                found = Some(p);
                                break;
                            }
                        }
                        let p = found.ok_or_else(|| err(format!("include file '{inc}' not found"), line))?;
                        let t = self.process_file(&p)?;
                        // keep it on one logical line position: replace newlines with spaces
                        // is wrong for `define bodies; just append (line numbers shift after includes)
                        out.push_str(&t);
                    }
                    "timescale" => {
                        let st = i;
                        while i < s.len() && s[i] != '\n' {
                            i += 1;
                        }
                        let spec: String = s[st..i].iter().collect();
                        let (u, p) = parse_timescale(&spec).ok_or_else(|| err(format!("bad `timescale '{}'", spec.trim()), line))?;
                        out.push_str(&format!(" __fx_timescale {} {} ; ", u, p));
                    }
                    "default_nettype" | "celldefine" | "endcelldefine" | "resetall"
                    | "unconnected_drive" | "nounconnected_drive" | "pragma" | "line" | "begin_keywords"
                    | "end_keywords" => {
                        while i < s.len() && s[i] != '\n' {
                            i += 1;
                        }
                    }
                    "__LINE__" => out.push_str(&line.to_string()),
                    "__FILE__" => out.push_str(&format!("\"{fname}\"")),
                    _ => {
                        let m = self
                            .macros
                            .get(&name)
                            .cloned()
                            .ok_or_else(|| err(format!("undefined macro `{name}"), line))?;
                        let mut body = m.body.clone();
                        if let Some(params) = &m.params {
                            while i < s.len() && s[i].is_whitespace() && s[i] != '\n' {
                                i += 1;
                            }
                            if i >= s.len() || s[i] != '(' {
                                return Err(err(format!("macro `{name} requires arguments"), line));
                            }
                            i += 1;
                            let mut args = Vec::new();
                            let mut cur = String::new();
                            let mut depth = 0;
                            while i < s.len() {
                                let ch = s[i];
                                if ch == '\n' {
                                    line += 1;
                                }
                                if (ch == ',' || ch == ')') && depth == 0 {
                                    args.push(cur.trim().to_string());
                                    cur.clear();
                                    i += 1;
                                    if ch == ')' {
                                        break;
                                    }
                                    continue;
                                }
                                if ch == '(' || ch == '[' || ch == '{' {
                                    depth += 1;
                                }
                                if ch == ')' || ch == ']' || ch == '}' {
                                    depth -= 1;
                                }
                                cur.push(ch);
                                i += 1;
                            }
                            body = substitute_params(&body, params, &args);
                        } else {
                            body = substitute_params(&body, &[], &[]);
                        }
                        let expanded = self.process_text(&body, fname, dir)?;
                        out.push_str(&expanded);
                    }
                }
                continue;
            }
            if active(&conds) {
                out.push(c);
            }
            i += 1;
        }
        if !conds.is_empty() {
            return Err(err("unterminated `ifdef".into(), line));
        }
        Ok(out)
    }
}

fn read_ident(s: &[char], i: &mut usize) -> String {
    while *i < s.len() && (s[*i] == ' ' || s[*i] == '\t') {
        *i += 1;
    }
    let st = *i;
    while *i < s.len() && (s[*i].is_alphanumeric() || s[*i] == '_' || s[*i] == '$') {
        *i += 1;
    }
    s[st..*i].iter().collect()
}

fn substitute_params(body: &str, params: &[String], args: &[String]) -> String {
    let b: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_alphabetic() || b[i] == '_' {
            let st = i;
            while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_' || b[i] == '$') {
                i += 1;
            }
            let w: String = b[st..i].iter().collect();
            if let Some(k) = params.iter().position(|p| *p == w) {
                out.push_str(args.get(k).map(|s| s.as_str()).unwrap_or(""));
            } else {
                out.push_str(&w);
            }
        } else if b[i] == '`' && i + 1 < b.len() && b[i + 1] == '`' {
            i += 2; // token pasting
        } else if b[i] == '`' && i + 1 < b.len() && b[i + 1] == '"' {
            out.push('"'); // `" : a string with macro arguments substituted
            i += 2;
        } else if b[i..].starts_with(&['`', '\\', '`', '"']) {
            out.push_str("\\\""); // `\`" : an escaped quote inside `"...`"
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// `1ns / 1ps` -> (-9, -12): powers of ten of seconds
fn parse_timescale(spec: &str) -> Option<(i32, i32)> {
    let mut parts = spec.split('/');
    let one = |t: &str| -> Option<i32> {
        let t: String = t.chars().filter(|c| !c.is_whitespace()).collect();
        let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
        let unit = &t[digits.len()..];
        let mag = match digits.as_str() {
            "1" => 0,
            "10" => 1,
            "100" => 2,
            _ => return None,
        };
        let e = match unit {
            "s" => 0,
            "ms" => -3,
            "us" => -6,
            "ns" => -9,
            "ps" => -12,
            "fs" => -15,
            _ => return None,
        };
        Some(e + mag)
    };
    let u = one(parts.next()?)?;
    let p = one(parts.next()?)?;
    Some((u, p.min(u)))
}
