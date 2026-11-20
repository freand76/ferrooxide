//! A small Verilog preprocessor: `define (with arguments), `undef, `ifdef/`ifndef/`elsif/`else/`endif,
//! `include, `line (for `__LINE__ / `__FILE__ only, not message locations). Other directives
//! (`timescale, `default_nettype, ...) are dropped.
//! Comments are stripped; line structure is preserved (directive lines become empty lines).

use crate::{Error, LineSeg, Options, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
struct Macro {
    params: Option<Vec<String>>,
    /// default values of the parameters (`define M(a, b = 1)`)
    defaults: Vec<Option<String>>,
    body: String,
}

pub struct Preprocessor {
    macros: HashMap<String, Macro>,
    incdirs: Vec<PathBuf>,
    depth: usize,
    /// while a macro body is expanded: the line of the (outermost) invocation, for `__LINE__
    macro_line: Option<u32>,
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
        macros.insert("__ICARUS__".to_string(), Macro { params: None, defaults: Vec::new(), body: "1".into() });
        for (k, v) in &opts.defines {
            macros.insert(k.clone(), Macro { params: None, defaults: Vec::new(), body: v.clone() });
        }
        Preprocessor { macros, incdirs: opts.incdirs.clone(), depth: 0, macro_line: None }
    }

    pub fn process_file(&mut self, path: &Path) -> Result<String> {
        Ok(self.process_file_mapped(path)?.0)
    }

    /// The preprocessed text of a file and where its lines come from (includes).
    pub fn process_file_mapped(&mut self, path: &Path) -> Result<(String, Vec<LineSeg>)> {
        let bytes = std::fs::read(path).map_err(|e| Error {
            msg: format!("cannot read file: {e}"),
            file: path.display().to_string(),
            line: 0,
        })?;
        // a file that is not UTF-8 (e.g. Latin-1 comments) is read byte by byte: each byte is
        // the character of that code, so bytes in strings keep their values
        let text = String::from_utf8(bytes).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect());
        // (a file named without a directory is in `.`: its includes are `./name`, as the
        // reference names them)
        let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        self.process_text_mapped(&text, &path.display().to_string(), &dir)
    }

    pub fn process_text(&mut self, text: &str, fname: &str, dir: &Path) -> Result<String> {
        Ok(self.process_text_mapped(text, fname, dir)?.0)
    }

    fn process_text_mapped(&mut self, text: &str, fname: &str, dir: &Path) -> Result<(String, Vec<LineSeg>)> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(Error { msg: "include/macro nesting too deep".into(), file: fname.into(), line: 0 });
        }
        let r = self.run(text, fname, dir);
        self.depth -= 1;
        r
    }

    fn run(&mut self, text: &str, fname: &str, dir: &Path) -> Result<(String, Vec<LineSeg>)> {
        let s: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        // where the output lines come from: this file, except for included text and
        // expansions whose line count differs from that of their source
        let mut segs = vec![LineSeg { start: 1, file: fname.to_string(), delta: 0 }];
        // the output's line number at its end (newlines counted up to `counted`)
        let (mut counted, mut out_line) = (0usize, 1u32);
        let mut out_line_now = |out: &str| {
            out_line += out[counted..].matches('\n').count() as u32;
            counted = out.len();
            out_line
        };
        // from output line `at` on: this file's line `src`
        let resync = |segs: &mut Vec<LineSeg>, at: u32, src: u32| {
            let delta = src as i64 - at as i64;
            if segs.last().is_some_and(|l| l.file == fname && l.delta == delta) {
                return;
            }
            if segs.last().is_some_and(|l| l.start == at) {
                segs.pop();
            }
            segs.push(LineSeg { start: at, file: fname.to_string(), delta });
        };
        let mut i = 0;
        let mut line = 1u32;
        // `line N "file" level: the next line is line N of `file` for `__LINE__ / `__FILE__
        let mut line_off = 0i64;
        let mut cur_file = fname.to_string();
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
                // `\name : an escaped identifier (to the next white space) names a macro too
                let name: String = if s.get(i) == Some(&'\\') {
                    escaped_ident(&s, &mut i)
                } else {
                    let st = i;
                    while i < s.len() && (s[i].is_alphanumeric() || s[i] == '_' || s[i] == '$') {
                        i += 1;
                    }
                    s[st..i].iter().collect()
                };
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
                        while i < s.len() && (s[i] == ' ' || s[i] == '\t') {
                            i += 1;
                        }
                        // an escaped name ends at white space, so its parameters follow it
                        let escaped = s.get(i) == Some(&'\\');
                        let id = if escaped { escaped_ident(&s, &mut i) } else { read_ident(&s, &mut i) };
                        if escaped && s.get(i) == Some(&' ') && s.get(i + 1) == Some(&'(') {
                            i += 1;
                        }
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
                            // (a continued line ends in a newline in the expansion, IEEE 1800
                            // 22.5.1: directives in the macro text end there)
                            if s[i] == '\\' && i + 1 < s.len() && s[i + 1] == '\n' {
                                body.push('\n');
                                out.push('\n');
                                line += 1;
                                i += 2;
                                continue;
                            }
                            if s[i] == '/' && i + 1 < s.len() && s[i + 1] == '/' {
                                while i < s.len() && s[i] != '\n' {
                                    i += 1;
                                }
                                // a line comment ending in a backslash: the macro continues
                                if i < s.len() && i > 0 && s[i - 1] == '\\' {
                                    body.push('\n');
                                    out.push('\n');
                                    line += 1;
                                    i += 1;
                                    continue;
                                }
                                break;
                            }
                            if s[i] == '/' && i + 1 < s.len() && s[i + 1] == '*' {
                                // a block comment (may span continued lines): a space
                                i += 2;
                                while i < s.len() && !(s[i] == '*' && i + 1 < s.len() && s[i + 1] == '/') {
                                    if s[i] == '\n' {
                                        out.push('\n');
                                        line += 1;
                                    }
                                    i += 1;
                                }
                                i += 2;
                                body.push(' ');
                                continue;
                            }
                            body.push(s[i]);
                            i += 1;
                        }
                        // `name = default` (IEEE 1800 22.5.1)
                        let mut defaults = Vec::new();
                        let params = params.map(|ps| {
                            ps.into_iter()
                                .map(|p| match p.split_once('=') {
                                    Some((n, d)) => {
                                        defaults.push(Some(d.trim().to_string()));
                                        n.trim().to_string()
                                    }
                                    None => {
                                        defaults.push(None);
                                        p
                                    }
                                })
                                .collect()
                        });
                        if DIRECTIVES.contains(&id.as_str()) {
                            return Err(err(format!("`{id} is a compiler directive and cannot be a macro name"), line));
                        }
                        if open_string(&body) {
                            return Err(err(format!("the text of macro `{id} has a string literal that does not end"), line));
                        }
                        self.macros.insert(id, Macro { params, defaults, body: body.trim().to_string() });
                    }
                    // IEEE 1800 22.5.3: every text macro is gone
                    "undefineall" => self.macros.clear(),
                    "undef" => {
                        let id = read_ident(&s, &mut i);
                        self.macros.remove(&id);
                    }
                    "include" => {
                        while i < s.len() && (s[i] == ' ' || s[i] == '\t') {
                            i += 1;
                        }
                        let inc: String = if i < s.len() && s[i] == '`' {
                            // `include `NAME: the file name is what the macro expands to
                            let st = i;
                            while i < s.len() && s[i] != '\n' && !(s[i] == '/' && i + 1 < s.len() && s[i + 1] == '/') {
                                i += 1;
                            }
                            let text: String = s[st..i].iter().collect();
                            let x = self.process_text(&text, &cur_file, dir)?;
                            let x = x.trim();
                            match x.strip_prefix('"').and_then(|r| r.strip_suffix('"')).or_else(|| x.strip_prefix('<').and_then(|r| r.strip_suffix('>'))) {
                                Some(n) => n.to_string(),
                                None => return Err(err(format!("`include: '{x}' is not a file name in quotes"), line)),
                            }
                        } else {
                            let close = if i < s.len() && s[i] == '<' { '>' } else { '"' };
                            i += 1;
                            let st = i;
                            while i < s.len() && s[i] != close && s[i] != '\n' {
                                i += 1;
                            }
                            let inc: String = s[st..i].iter().collect();
                            i += 1;
                            inc
                        };
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
                        let saved = self.macro_line.take();
                        let t = self.process_file_mapped(&p);
                        self.macro_line = saved;
                        let (t, inner) = t?;
                        // the included lines keep their own file and line numbers
                        let start = out_line_now(&out);
                        if segs.last().is_some_and(|l| l.start == start) {
                            segs.pop();
                        }
                        for g in inner {
                            segs.push(LineSeg { start: g.start + start - 1, file: g.file, delta: g.delta - (start as i64 - 1) });
                        }
                        out.push_str(&t);
                        let end = out_line_now(&out);
                        // (the rest of the include's line is on the included text's last
                        // line; the next output line is this file's next line)
                        segs.push(LineSeg { start: end + 1, file: fname.to_string(), delta: line as i64 - end as i64 });
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
                    "resetall" => {
                        // back to the default timescale (1s / 1s)
                        if active(&conds) {
                            out.push_str(" __fx_timescale 0 0 ; ");
                        }
                    }
                    "default_nettype" | "unconnected_drive" => {
                        // the argument may follow comments and line breaks
                        skip_space_comments(&s, &mut i, &mut line, &mut out);
                        let t = read_ident(&s, &mut i);
                        if name == "default_nettype" && active(&conds) {
                            out.push_str(&format!(" __fx_nettype {t} ; "));
                        }
                        if name == "unconnected_drive" && active(&conds) {
                            out.push_str(&format!(" __fx_uncdrive {t} ; "));
                        }
                    }
                    "begin_keywords" => {
                        skip_space_comments(&s, &mut i, &mut line, &mut out);
                        if i < s.len() && s[i] == '"' {
                            i += 1;
                            let st = i;
                            while i < s.len() && s[i] != '"' && s[i] != '\n' {
                                i += 1;
                            }
                            let v: String = s[st..i].iter().collect();
                            i += 1;
                            if active(&conds) {
                                out.push_str(&format!(" __fx_keywords \"{v}\" ; "));
                            }
                        }
                    }
                    "end_keywords" => {
                        if active(&conds) {
                            out.push_str(" __fx_keywords ; ");
                        }
                    }
                    // no arguments
                    "nounconnected_drive" => {
                        if active(&conds) {
                            out.push_str(" __fx_uncdrive none ; ");
                        }
                    }
                    "celldefine" | "endcelldefine" => {}
                    "line" => {
                        let st = i;
                        while i < s.len() && s[i] != '\n' {
                            i += 1;
                        }
                        let rest: String = s[st..i].iter().collect();
                        let rest = rest.split("//").next().unwrap_or_default();
                        // `line number "filename" level (IEEE 1800-2017 22.12)
                        let mut parts = rest.trim().splitn(2, char::is_whitespace);
                        let n: Option<i64> = parts.next().and_then(|n| n.parse().ok()).filter(|&n| n >= 0);
                        let fl = parts.next().map(str::trim).and_then(|r| r.strip_prefix('"')).and_then(|r| r.split_once('"'));
                        let level = fl.map(|x| x.1.trim());
                        let (Some(n), Some((f, _)), Some("0" | "1" | "2")) = (n, fl, level) else {
                            return Err(err("`line needs a line number, a quoted file name and a level (0, 1 or 2)".into(), line));
                        };
                        if active(&conds) {
                            line_off = n - (line as i64 + 1);
                            cur_file = f.to_string();
                        }
                    }
                    "pragma" => {
                        let st = i;
                        while i < s.len() && s[i] != '\n' {
                            i += 1;
                        }
                        let rest: String = s[st..i].iter().collect();
                        if rest.split("//").next().unwrap_or_default().trim().is_empty() {
                            return Err(err("`pragma needs a pragma name".into(), line));
                        }
                    }
                    "__LINE__" => out.push_str(&self.macro_line.unwrap_or((line as i64 + line_off) as u32).to_string()),
                    "__FILE__" => out.push_str(&format!("\"{cur_file}\"")),
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
                                // a string literal is one piece (commas and parentheses in it)
                                if ch == '"' {
                                    cur.push(ch);
                                    i += 1;
                                    while i < s.len() && s[i] != '"' {
                                        if s[i] == '\\' && i + 1 < s.len() {
                                            cur.push(s[i]);
                                            i += 1;
                                        }
                                        cur.push(s[i]);
                                        i += 1;
                                    }
                                    if i < s.len() {
                                        cur.push(s[i]);
                                        i += 1;
                                    }
                                    continue;
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
                            // `M()` of a macro without parameters: no arguments
                            if params.is_empty() && args.len() == 1 && args[0].is_empty() {
                                args.clear();
                            }
                            if args.len() > params.len() {
                                return Err(err(format!("macro `{name} takes {} arguments, given {}", params.len(), args.len()), line));
                            }
                            // (IEEE 1800-2017 22.5.1: a missing argument needs a default)
                            if let Some(k) = (args.len()..params.len()).find(|&k| m.defaults.get(k).is_none_or(|d| d.is_none())) {
                                return Err(err(format!("macro `{name}: no argument for '{}', which has no default", params[k]), line));
                            }
                            // missing or empty arguments take their defaults
                            for (k, d) in m.defaults.iter().enumerate() {
                                if let Some(d) = d {
                                    if args.len() <= k {
                                        args.resize(k + 1, String::new());
                                    }
                                    if args[k].is_empty() {
                                        args[k] = d.clone();
                                    }
                                }
                            }
                            body = substitute_params(&body, params, &args);
                        } else {
                            body = substitute_params(&body, &[], &[]);
                        }
                        let outer = self.macro_line.is_none();
                        if outer {
                            self.macro_line = Some((line as i64 + line_off) as u32);
                        }
                        let expanded = self.process_text(&body, &cur_file, dir);
                        if outer {
                            self.macro_line = None;
                        }
                        out.push_str(&expanded?);
                        // (an expansion may have more or fewer lines than its source)
                        let at = out_line_now(&out);
                        resync(&mut segs, at, line);
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
        Ok((out, segs))
    }
}

/// Skip spaces, line breaks and comments (line breaks are kept in `out` for line numbers).
fn skip_space_comments(s: &[char], i: &mut usize, line: &mut u32, out: &mut String) {
    while *i < s.len() {
        if s[*i] == '\n' {
            out.push('\n');
            *line += 1;
            *i += 1;
        } else if s[*i].is_whitespace() {
            *i += 1;
        } else if s[*i] == '/' && *i + 1 < s.len() && s[*i + 1] == '/' {
            while *i < s.len() && s[*i] != '\n' {
                *i += 1;
            }
        } else if s[*i] == '/' && *i + 1 < s.len() && s[*i + 1] == '*' {
            *i += 2;
            while *i < s.len() && !(s[*i] == '*' && *i + 1 < s.len() && s[*i + 1] == '/') {
                if s[*i] == '\n' {
                    out.push('\n');
                    *line += 1;
                }
                *i += 1;
            }
            *i += 2;
        } else {
            break;
        }
    }
}

/// An escaped identifier `\name` at `*i`: the characters up to white space (which ends it
/// and is not part of it), without the backslash.
fn escaped_ident(s: &[char], i: &mut usize) -> String {
    *i += 1;
    let st = *i;
    while *i < s.len() && !s[*i].is_whitespace() {
        *i += 1;
    }
    s[st..*i].iter().collect()
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

/// The compiler directives (IEEE 1800-2017 22), which cannot be redefined as macros.
const DIRECTIVES: &[&str] = &[
    "begin_keywords", "celldefine", "default_nettype", "define", "else", "elsif", "end_keywords", "endcelldefine", "endif",
    "ifdef", "ifndef", "include", "line", "nounconnected_drive", "pragma", "resetall", "timescale", "unconnected_drive",
    "undef", "undefineall", "__FILE__", "__LINE__",
];

/// Whether macro text `t` has a string literal that does not end in it (outside strings,
/// the `" of a stringified argument and an escaped `\`" do not count).
fn open_string(t: &str) -> bool {
    let b = t.as_bytes();
    let (mut open, mut k) = (false, 0);
    while k < b.len() {
        match b[k] {
            // (in a string literal a backtick is a plain character)
            b'`' if !open && b.get(k + 1) == Some(&b'"') => k += 1,
            b'`' if !open && b.get(k + 1) == Some(&b'\\') && b.get(k + 2) == Some(&b'`') => k += 3,
            b'\\' if open => k += 1,
            b'"' => open = !open,
            _ => {}
        }
        k += 1;
    }
    open
}
