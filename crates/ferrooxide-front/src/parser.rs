//! Recursive-descent parser for Verilog-2005 plus common SystemVerilog constructs.

use crate::ast::*;
use crate::lexer::{Tok, Token};
use crate::svtypes::{Package, SvType};
use crate::{Error, Result};
use std::collections::HashMap;
use std::rc::Rc;

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    file: String,
    /// `timescale in effect (carried from file to file by the caller)
    pub timescale: Option<(i32, i32)>,
    pub default_nettype: Option<String>,
    pub unconnected_drive: Option<bool>,
    /// counter for names of unnamed instances
    unnamed: u32,
    /// typedefs in scope (file scope and packages carry over to the next file)
    pub types: HashMap<String, Rc<SvType>>,
    /// variables of the current module with a typedef'd or struct type: (type, unpacked dims)
    pub(crate) vars: HashMap<String, (Rc<SvType>, usize)>,
    /// the data type of the declaration being parsed
    pub(crate) decl_type: Option<Rc<SvType>>,
    /// the type selected by the last lvalue (for an assignment pattern on the right)
    pub(crate) last_type: Option<SvType>,
    /// items produced while parsing a declaration (enum constants)
    pub(crate) pending: Vec<Item>,
    /// items at file scope ($unit): added to every module that follows
    pub unit_items: Vec<Item>,
    /// names read so far that are SystemVerilog keywords (lint)
    kw_names: Vec<(String, Line)>,
    /// packages (carried over to the next file)
    pub packages: HashMap<String, Rc<Package>>,
    /// packages the current module (or package) imports or refers to
    pub(crate) mod_pkgs: Vec<String>,
    /// the return type of the function being parsed (for `return '{...}`)
    ret_type: Option<Rc<SvType>>,
    /// modports of the interface being parsed
    modports: Vec<String>,
}

struct PType {
    signed: bool,
    integer: bool,
    real: bool,
    range: Option<Range>,
    /// a typedef'd type (for assignment patterns)
    sv: Option<SvType>,
}

const KEYWORDS: &[&str] = &[
    "module", "endmodule", "macromodule", "input", "output", "inout", "wire", "reg", "integer", "time", "real",
    "parameter", "localparam", "assign", "always", "initial", "begin", "end", "if", "else", "case", "casez",
    "casex", "endcase", "default", "for", "while", "repeat", "forever", "posedge", "negedge", "or", "function",
    "endfunction", "task", "endtask", "generate", "endgenerate", "genvar", "signed", "unsigned", "fork", "join",
    "wait", "disable", "tri", "supply0", "supply1", "wand", "wor", "logic", "bit", "int", "byte", "shortint",
    "longint", "always_comb", "always_ff", "always_latch", "automatic", "return", "do", "final", "defparam",
    "specify", "endspecify", "specparam", "primitive", "endprimitive", "table", "endtable", "join_any", "force", "release", "deassign", "realtime", "event", "join_none", "break", "continue", "unique", "priority",
];

fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

/// SystemVerilog keywords that are ordinary identifiers in Verilog-2005 code: accepted as names
/// wherever an identifier is required.
const SV_ONLY: &[&str] = &[
    "logic", "bit", "int", "byte", "shortint", "longint", "always_comb", "always_ff", "always_latch", "return", "do",
    "final", "join_any", "join_none", "break", "continue", "unique", "priority", "var",
];

/// Keywords IEEE 1800 (SystemVerilog) reserves beyond those of Verilog-2005: legal names in
/// Verilog-2005 code, but such code does not compile as SystemVerilog (lint check).
pub const SV_RESERVED: &[&str] = &[
    "accept_on", "alias", "always_comb", "always_ff", "always_latch", "assert", "assume", "before", "bind", "bins",
    "binsof", "bit", "break", "byte", "chandle", "checker", "class", "clocking", "const", "constraint", "context",
    "continue", "cover", "covergroup", "coverpoint", "cross", "dist", "do", "endchecker", "endclass", "endclocking",
    "endgroup", "endinterface", "endpackage", "endprogram", "endproperty", "endsequence", "enum", "eventually",
    "expect", "export", "extends", "extern", "final", "first_match", "foreach", "forkjoin", "global", "iff",
    "ignore_bins", "illegal_bins", "implements", "implies", "import", "inside", "int", "interconnect", "interface",
    "intersect", "join_any", "join_none", "let", "local", "logic", "longint", "matches", "modport", "nettype", "new",
    "nexttime", "null", "package", "packed", "priority", "program", "property", "protected", "pure", "rand", "randc",
    "randcase", "randsequence", "ref", "reject_on", "restrict", "return", "s_always", "s_eventually", "s_nexttime",
    "s_until", "s_until_with", "sequence", "shortint", "shortreal", "soft", "solve", "static", "string", "strong",
    "struct", "super", "sync_accept_on", "sync_reject_on", "tagged", "this", "throughout", "timeprecision",
    "timeunit", "type", "typedef", "union", "unique", "unique0", "until", "until_with", "untyped", "var", "virtual",
    "void", "wait_order", "weak", "wildcard", "with", "within",
];

fn is_sv_only(s: &str) -> bool {
    SV_ONLY.contains(&s)
}

const GATES: &[&str] = &[
    "and", "or", "nand", "nor", "xor", "xnor", "not", "buf", "bufif0", "bufif1", "notif0", "notif1", "nmos", "pmos",
    "rnmos", "rpmos", "cmos", "rcmos", "pullup", "pulldown", "tran", "rtran", "tranif0", "tranif1", "rtranif0",
    "rtranif1",
];

const STRENGTHS: &[&str] = &[
    "supply0", "supply1", "strong0", "strong1", "pull0", "pull1", "weak0", "weak1", "highz0", "highz1", "small",
    "medium", "large",
];

impl Parser {
    pub fn new(toks: Vec<Token>, file: String) -> Self {
        Parser {
            toks,
            pos: 0,
            file,
            timescale: None,
            default_nettype: None,
            unconnected_drive: None,
            unnamed: 0,
            types: HashMap::new(),
            vars: HashMap::new(),
            decl_type: None,
            last_type: None,
            pending: Vec::new(),
            unit_items: Vec::new(),
            kw_names: Vec::new(),
            packages: HashMap::new(),
            mod_pkgs: Vec::new(),
            ret_type: None,
            modports: Vec::new(),
        }
    }

    // ---------- token helpers ----------
    pub(crate) fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    pub(crate) fn peek_at(&self, k: usize) -> &Tok {
        let i = (self.pos + k).min(self.toks.len() - 1);
        &self.toks[i].tok
    }
    pub(crate) fn line(&self) -> u32 {
        self.toks[self.pos].line
    }
    pub(crate) fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    pub(crate) fn err<T>(&self, msg: impl Into<String>) -> Result<T> {
        Err(Error { msg: msg.into(), file: self.file.clone(), line: self.line() })
    }
    pub(crate) fn is_p(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }
    pub(crate) fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Tok::Ident(q) if q == k)
    }
    pub(crate) fn eat_p(&mut self, p: &str) -> bool {
        if self.is_p(p) {
            self.bump();
            true
        } else {
            false
        }
    }
    pub(crate) fn eat_kw(&mut self, k: &str) -> bool {
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
    pub(crate) fn expect_kw(&mut self, k: &str) -> Result<()> {
        if self.eat_kw(k) {
            Ok(())
        } else {
            self.err(format!("expected '{k}', found {}", self.describe()))
        }
    }
    pub(crate) fn describe(&self) -> String {
        match self.peek() {
            Tok::Ident(s) => format!("'{s}'"),
            Tok::SysIdent(s) => format!("'{s}'"),
            Tok::Num(_) | Tok::Real(_) => "number".into(),
            Tok::Str(_) => "string".into(),
            Tok::Punct(p) => format!("'{p}'"),
            Tok::Eof => "end of file".into(),
        }
    }
    pub(crate) fn ident(&mut self) -> Result<String> {
        match self.peek().clone() {
            Tok::Ident(s) if !is_keyword(&s) || is_sv_only(&s) => {
                // (a name after '.' belongs to another scope: a port of an instance, a
                // hierarchical reference)
                let after_dot = self.pos > 0 && matches!(self.toks[self.pos - 1].tok, Tok::Punct("."));
                if SV_RESERVED.contains(&s.as_str()) && !after_dot {
                    self.kw_names.push((s.clone(), self.line()));
                }
                self.bump();
                Ok(s)
            }
            _ => self.err(format!("expected identifier, found {}", self.describe())),
        }
    }
    /// Any identifier, keywords included (`default_nettype wire).
    /// Skip a balanced `( ... )`.
    fn skip_parens(&mut self) -> Result<()> {
        let mut depth = 0;
        loop {
            match self.bump() {
                Tok::Punct("(") => depth += 1,
                Tok::Punct(")") => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                Tok::Eof => return self.err("unbalanced parentheses"),
                _ => {}
            }
        }
    }

    /// `(sources =>|*> destinations) = delays;` in a specify block; None if it is not a path.
    fn spec_path(&mut self, line: Line) -> Result<Option<Item>> {
        // the tokens of the parenthesised path
        self.bump();
        let mut toks = Vec::new();
        let mut depth = 0;
        loop {
            let t = self.bump();
            match t {
                Tok::Punct("(") => depth += 1,
                Tok::Punct(")") if depth == 0 => break,
                Tok::Punct(")") => depth -= 1,
                Tok::Eof => return self.err("unterminated specify path"),
                _ => {}
            }
            toks.push(t);
        }
        // the arrow: => *> (also +=> -=> and the full-connection forms)
        let arrow = (0..toks.len().saturating_sub(1)).find(|&i| {
            matches!(toks[i], Tok::Punct("=" | "*" | "+=" | "-=")) && matches!(toks[i + 1], Tok::Punct(">"))
        });
        let names = |ts: &[Tok]| -> Vec<String> {
            let mut v = Vec::new();
            let mut br = 0;
            for t in ts {
                match t {
                    Tok::Punct("[") => br += 1,
                    Tok::Punct("]") => br -= 1,
                    Tok::Ident(s) if br == 0 && !matches!(s.as_str(), "posedge" | "negedge" | "edge") => v.push(s.clone()),
                    _ => {}
                }
            }
            v
        };
        let Some(ai) = arrow else {
            while !self.eat_p(";") {
                if matches!(self.peek(), Tok::Eof) {
                    break;
                }
                self.bump();
            }
            return Ok(None);
        };
        let srcs = names(&toks[..ai]);
        let rest = &toks[ai + 2..];
        // data path `(q +: d)`: the destination is the first name
        let dsts = if matches!(rest.first(), Some(Tok::Punct("("))) {
            names(rest).into_iter().take(1).collect()
        } else {
            names(rest)
        };
        self.expect_p("=")?;
        let delay = if self.is_p("(") {
            self.delay_value()?
        } else {
            self.primary()?
        };
        // more than three delays (6 or 12 transitions): rise, fall, turn-off are the first three
        let delay = match delay {
            Expr::SysCall(n, mut v) if n == "$__delays" && v.len() > 3 => {
                v.truncate(3);
                Expr::SysCall(n, v)
            }
            d => d,
        };
        self.expect_p(";")?;
        Ok(Some(Item::SpecPath { srcs, dsts, delay, line }))
    }

    fn ident_or_kw(&mut self) -> Result<String> {
        match self.bump() {
            Tok::Ident(s) => Ok(s),
            _ => self.err("expected a name"),
        }
    }

    /// a or a.b.c (hierarchical name as a dotted string)
    fn dotted_name(&mut self) -> Result<String> {
        let mut n = self.ident()?;
        while self.eat_p(".") {
            n.push('.');
            n.push_str(&self.ident()?);
        }
        Ok(n)
    }

    pub(crate) fn opt_block_label(&mut self) -> Result<Option<String>> {
        if self.eat_p(":") {
            Ok(Some(self.ident()?))
        } else {
            Ok(None)
        }
    }

    // ---------- source ----------
    pub fn parse_source(&mut self) -> Result<Vec<Module>> {
        let mut mods = Vec::new();
        loop {
            match self.peek() {
                Tok::Eof => break,
                Tok::Ident(s) if s == "module" || s == "macromodule" || s == "interface" => mods.push(self.module()?),
                Tok::Ident(s) if s == "primitive" => mods.push(self.primitive()?),
                // file-scope ($unit) declarations: visible in the modules that follow
                Tok::Ident(s) if s == "package" => self.package()?,
                Tok::Ident(s) if s == "import" => self.import(true)?,
                Tok::Ident(s) if matches!(s.as_str(), "typedef" | "localparam" | "parameter" | "function" | "task") => {
                    let mut v = Vec::new();
                    self.module_item(&mut v)?;
                    self.unit_items.extend(v);
                }
                Tok::Ident(s) if s == "__fx_uncdrive" => {
                    self.bump();
                    let t = self.ident_or_kw()?;
                    self.expect_p(";")?;
                    self.unconnected_drive = match t.as_str() {
                        "pull1" => Some(true),
                        "pull0" => Some(false),
                        _ => None,
                    };
                }
                Tok::Ident(s) if s == "__fx_nettype" => {
                    self.bump();
                    let t = self.ident_or_kw()?;
                    self.expect_p(";")?;
                    self.default_nettype = if t == "wire" { None } else { Some(t) };
                }
                Tok::Ident(s) if s == "__fx_timescale" => {
                    self.bump();
                    let num = |p: &mut Parser| -> Result<i32> {
                        let neg = p.eat_p("-");
                        match p.bump() {
                            Tok::Num(n) => {
                                let v: i32 = n.digits.parse().unwrap_or(0);
                                Ok(if neg { -v } else { v })
                            }
                            _ => p.err("bad timescale"),
                        }
                    };
                    let u = num(self)?;
                    let pr = num(self)?;
                    self.expect_p(";")?;
                    self.timescale = Some((u, pr));
                }
                Tok::Punct(";") => {
                    self.bump();
                }
                _ => return self.err(format!("expected 'module', found {}", self.describe())),
            }
        }
        Ok(mods)
    }

    fn module(&mut self) -> Result<Module> {
        let line = self.line();
        let interface = self.is_kw("interface");
        let end_kw = if interface { "endinterface" } else { "endmodule" };
        self.bump();
        let name = self.ident()?;
        let mut iface_ports = Vec::new();
        self.modports.clear();
        // typedefs of the module stay in it
        let saved_types = self.types.clone();
        self.vars.clear();
        self.mod_pkgs.clear();
        let mut items = Vec::new();
        // imports in the module header
        while self.is_kw("import") {
            self.import(false)?;
        }
        let mut ports = Vec::new();
        // parameter port list
        if self.eat_p("#") {
            self.expect_p("(")?;
            let mut local = false;
            let mut pt = PType { signed: false, integer: false, real: false, range: None, sv: None };
            loop {
                if self.eat_kw("parameter") {
                    local = false;
                    pt = self.param_type()?;
                } else if self.eat_kw("localparam") {
                    local = true;
                    pt = self.param_type()?;
                } else if matches!(self.peek(), Tok::Ident(s) if s == "integer" || s == "signed" || s == "int" || s == "logic" || s == "bit" || s == "real")
                    || self.is_p("[")
                    || self.is_sv_type_start()
                {
                    pt = self.param_type()?;
                }
                let pline = self.line();
                let pname = self.ident()?;
                self.expect_p("=")?;
                let value = self.expr()?;
                let value = match &pt.sv {
                    Some(t) => self.resolve_pattern(value, t)?,
                    None => value,
                };
                self.decl_type = pt.sv.clone().map(Rc::new);
                self.declare_var(&pname, 0);
                items.append(&mut self.pending);
                items.push(Item::Param(ParamDecl {
                    local,
                    signed: pt.signed,
                    range: pt.range.clone(),
                    integer: pt.integer,
                    real: pt.real,
                    name: pname,
                    value,
                    line: pline,
                }));
                if !self.eat_p(",") {
                    break;
                }
            }
            self.expect_p(")")?;
        }
        // port list
        if self.eat_p("(") {
            if !self.is_p(")") {
                if matches!(self.peek(), Tok::Ident(s) if s == "input" || s == "output" || s == "inout") || self.is_iface_port() {
                    // ANSI style
                    let mut cur: Option<Decl> = None;
                    loop {
                        let pline = self.line();
                        if self.is_iface_port() {
                            // interface port: bus_if b / bus_if.master b
                            let iface = self.ident()?;
                            if self.eat_p(".") {
                                self.ident()?;
                            }
                            let pname = self.ident()?;
                            iface_ports.push((pname.clone(), iface));
                            ports.push(pname);
                            cur = None;
                            if !self.eat_p(",") {
                                break;
                            }
                            continue;
                        }
                        if matches!(self.peek(), Tok::Ident(s) if s == "input" || s == "output" || s == "inout") {
                            let d = self.decl_head(true)?;
                            cur = Some(d);
                        } else if cur.is_none() {
                            return self.err("expected port direction");
                        } else if matches!(self.peek(), Tok::Ident(s) if is_keyword(s)) || self.is_p("[") || self.is_sv_type_start() {
                            // new type without direction: inherits direction
                            let dir = cur.as_ref().unwrap().dir;
                            let mut d = self.decl_head(false)?;
                            d.dir = dir;
                            cur = Some(d);
                        }
                        let pname = self.ident()?;
                        let mut dims = Vec::new();
                        while self.is_p("[") {
                            dims.push(self.range()?);
                        }
                        let init = if self.eat_p("=") { Some(self.expr()?) } else { None };
                        self.declare_var(&pname, dims.len());
                        items.append(&mut self.pending);
                        let mut d = cur.clone().unwrap();
                        d.line = pline;
                        d.names = vec![Declarator { name: pname.clone(), dims, init, line: pline }];
                        items.push(Item::Decl(d));
                        ports.push(pname);
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                } else {
                    loop {
                        if self.is_p(",") || self.is_p(")") {
                            ports.push(String::new());
                        } else {
                            ports.push(self.ident()?);
                        }
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                }
            }
            self.expect_p(")")?;
        }
        self.expect_p(";")?;
        while !self.is_kw(end_kw) {
            if matches!(self.peek(), Tok::Eof) {
                return self.err(format!("unexpected end of file in {}", if interface { "interface" } else { "module" }));
            }
            self.module_item(&mut items)?;
        }
        self.bump();
        self.opt_block_label()?;
        self.types = saved_types;
        // items of the packages it uses, then file-scope items, unless the module declares
        // the name itself
        let pkgs = std::mem::take(&mut self.mod_pkgs);
        let mut outer = self.package_items(&pkgs);
        outer.extend(self.unit_items.iter().cloned());
        if !outer.is_empty() {
            let mut own = declared_names(&items);
            let mut all: Vec<Item> = Vec::new();
            for it in outer {
                if item_name(&it).is_none_or(|n| own.insert(n.to_string())) {
                    all.push(it);
                }
            }
            all.extend(items);
            items = all;
        }
        Ok(Module {
            name,
            timescale: self.timescale,
            default_nettype: self.default_nettype.clone(),
            unconnected_drive: self.unconnected_drive,
            file: self.file.clone(),
            line,
            ports,
            items,
            udp: None,
            keyword_names: std::mem::take(&mut self.kw_names),
            interface,
            iface_ports,
            modports: std::mem::take(&mut self.modports),
        })
    }

    /// primitive name (ports); declarations; [initial q = v;] table ... endtable endprimitive
    fn primitive(&mut self) -> Result<Module> {
        let line = self.line();
        self.bump();
        let name = self.ident()?;
        let mut items = Vec::new();
        let mut ports = Vec::new();
        let mut sequential = false;
        self.expect_p("(")?;
        loop {
            if matches!(self.peek(), Tok::Ident(s) if s == "input" || s == "output") {
                let mut d = self.decl_head(true)?;
                if d.kind == NetKind::Reg {
                    sequential = true;
                }
                let dl = self.line();
                let n = self.ident()?;
                if self.eat_p("=") {
                    self.expr()?;
                }
                d.names = vec![Declarator { name: n.clone(), dims: vec![], init: None, line: dl }];
                items.push(Item::Decl(d));
                ports.push(n);
            } else {
                ports.push(self.ident()?);
            }
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        self.expect_p(";")?;
        let mut rows = Vec::new();
        let mut init = None;
        loop {
            if self.eat_kw("endprimitive") {
                break;
            }
            if matches!(self.peek(), Tok::Eof) {
                return self.err("unexpected end of file in primitive");
            }
            if self.eat_kw("initial") {
                self.ident()?;
                self.expect_p("=")?;
                init = Some(self.expr()?);
                self.expect_p(";")?;
            } else if self.eat_kw("table") {
                let raw = match self.bump() {
                    Tok::Str(s) => s,
                    _ => return self.err("bad table"),
                };
                self.expect_kw("endtable")?;
                for row in raw.split(';') {
                    let row = row.trim();
                    if row.is_empty() {
                        continue;
                    }
                    let parts: Vec<&str> = row.split(':').collect();
                    let syms = |t: &str| -> Vec<String> {
                        let mut v = Vec::new();
                        let cs: Vec<char> = t.chars().filter(|c| !c.is_whitespace()).collect();
                        let mut k = 0;
                        while k < cs.len() {
                            if cs[k] == '(' {
                                let e: String = cs[k..(k + 4).min(cs.len())].iter().collect();
                                v.push(e.to_ascii_lowercase());
                                k += 4;
                            } else {
                                v.push(cs[k].to_ascii_lowercase().to_string());
                                k += 1;
                            }
                        }
                        v
                    };
                    match parts.len() {
                        2 => rows.push((syms(parts[0]), None, parts[1].trim().to_ascii_lowercase())),
                        3 => rows.push((syms(parts[0]), Some(parts[1].trim().to_ascii_lowercase()), parts[2].trim().to_ascii_lowercase())),
                        _ => return self.err(format!("bad UDP table row '{row}'")),
                    }
                }
            } else if self.is_decl_start() {
                let mut d = self.decl_head(true)?;
                if d.kind == NetKind::Reg {
                    sequential = true;
                }
                d.names = self.declarators()?;
                items.push(Item::Decl(d));
            } else {
                return self.err(format!("unexpected {} in primitive", self.describe()));
            }
        }
        self.opt_block_label()?;
        let sequential = sequential || rows.iter().any(|r| r.1.is_some());
        Ok(Module {
            name,
            timescale: self.timescale,
            default_nettype: None,
            unconnected_drive: None,
            file: self.file.clone(),
            line,
            ports,
            items,
            udp: Some(UdpDef { sequential, rows, init }),
            keyword_names: std::mem::take(&mut self.kw_names),
            interface: false,
            iface_ports: vec![],
            modports: vec![],
        })
    }

    fn param_type(&mut self) -> Result<PType> {
        if self.is_sv_type_start() {
            let t = self.data_type()?.unwrap();
            let (kind, signed, range, packed) = t.decl_parts();
            // a parameter has one range: the whole width
            let range = if packed.is_empty() { range } else { Some((crate::svtypes::minus_one(t.width()), Expr::Num(dec(0)))) };
            let integer = matches!(kind, NetKind::Int(_) | NetKind::Integer) && range.is_none();
            return Ok(PType { signed, integer, real: false, range, sv: Some(t) });
        }
        let mut signed = false;
        let mut integer = false;
        let mut real = false;
        loop {
            if self.eat_kw("signed") {
                signed = true;
            } else if self.eat_kw("unsigned") {
            } else if self.eat_kw("integer") || self.eat_kw("int") {
                integer = true;
                signed = true;
            } else if self.eat_kw("real") || self.eat_kw("realtime") {
                real = true;
            } else if self.eat_kw("logic") || self.eat_kw("bit") || self.eat_kw("reg") {
            } else {
                break;
            }
        }
        let range = if self.is_p("[") { Some(self.range()?) } else { None };
        Ok(PType { signed, integer, real, range, sv: None })
    }

    pub(crate) fn range(&mut self) -> Result<Range> {
        self.expect_p("[")?;
        let a = self.expr()?;
        let r = if self.eat_p(":") {
            let b = self.expr()?;
            (a, b)
        } else {
            // SV [N] means [0:N-1]
            let n1 = Expr::Binary(BinOp::Sub, Box::new(a), Box::new(Expr::Num(dec(1))));
            (Expr::Num(dec(0)), n1)
        };
        self.expect_p("]")?;
        Ok(r)
    }

    /// Parses `[dir] [kind] [signed] [range]`.
    fn decl_head(&mut self, allow_dir: bool) -> Result<Decl> {
        let line = self.line();
        let mut dir = None;
        if allow_dir {
            if self.eat_kw("input") {
                dir = Some(Dir::Input)
            } else if self.eat_kw("output") {
                dir = Some(Dir::Output)
            } else if self.eat_kw("inout") {
                dir = Some(Dir::Inout)
            }
        }
        self.decl_type = None;
        if !self.sv_ident_here() {
            self.eat_kw("var");
        }
        if self.is_sv_type_start() {
            return self.typed_decl_head(dir, line);
        }
        let mut kind = NetKind::Wire;
        let mut explicit_kind = false;
        let mut signed = false;
        let mut net = NetType::Wire;
        let mut real_net = false;
        let mut strength = None;
        // `input bit;`: a SystemVerilog type name used as the port name
        let sv_name = dir.is_some() && self.sv_ident_here();
        if let Tok::Ident(s) = self.peek().clone() {
            if sv_name {
                return Ok(Decl {
                    dir,
                    kind,
                    net,
                    explicit_kind,
                    real_net: false,
                    strength: None,
                    delay: None,
                    signed,
                    range: None,
                    packed: vec![],
                    names: vec![],
                    line,
                });
            }
            net = match s.as_str() {
                "tri0" => NetType::Tri0,
                "tri1" => NetType::Tri1,
                "supply0" => NetType::Supply0,
                "supply1" => NetType::Supply1,
                "wand" | "triand" => NetType::Wand,
                "wor" | "trior" => NetType::Wor,
                "trireg" => NetType::Trireg,
                _ => NetType::Wire,
            };
            let k = match s.as_str() {
                "wire" | "tri" | "wand" | "wor" | "triand" | "trior" | "trireg" | "supply0" | "supply1" | "tri0"
                | "tri1" | "uwire" => Some(NetKind::Wire),
                "reg" | "logic" => Some(NetKind::Reg),
                "bit" => Some(NetKind::Bit),
                "integer" => Some(NetKind::Integer),
                "time" => Some(NetKind::Time),
                "genvar" => Some(NetKind::Genvar),
                "int" => Some(NetKind::Int(32)),
                "real" | "realtime" => Some(NetKind::Real),
                "event" => Some(NetKind::Event),
                "byte" => Some(NetKind::Int(8)),
                "shortint" => Some(NetKind::Int(16)),
                "longint" => Some(NetKind::Int(64)),
                _ => None,
            };
            if let Some(k) = k {
                self.bump();
                kind = k;
                explicit_kind = true;
                if matches!(k, NetKind::Integer | NetKind::Int(_)) {
                    signed = true;
                }
                // `wire logic` / `var logic` (`reg logic;`: logic is the name)
                if (self.is_kw("logic") || self.is_kw("reg")) && !self.sv_ident_here() {
                    self.bump();
                }
                // `wire real`: a real-valued net
                if kind == NetKind::Wire && self.eat_kw("real") {
                    kind = NetKind::Real;
                    real_net = true;
                }
                // drive / charge strength of a net declaration
                strength = self.strength()?;
            }
        }
        if self.eat_kw("signed") {
            signed = true;
        } else if self.eat_kw("unsigned") {
            signed = false;
        }
        let range = if self.is_p("[") { Some(self.range()?) } else { None };
        let mut packed = Vec::new();
        while range.is_some() && self.is_p("[") {
            packed.push(self.range()?);
        }
        // optional net delay: wire #5 w;
        let delay = if self.eat_p("#") { Some(self.delay_value()?) } else { None };
        // the type of a vector, for assignment patterns
        if let Some(r) = &range {
            let mut dims = vec![r.clone()];
            dims.extend(packed.iter().cloned());
            self.decl_type = Some(Rc::new(SvType::vector(kind, signed, dims)));
        }
        Ok(Decl { dir, kind, net, explicit_kind, real_net, strength, delay, signed, range, packed, names: Vec::new(), line })
    }

    fn declarators(&mut self) -> Result<Vec<Declarator>> {
        let mut v = Vec::new();
        loop {
            let line = self.line();
            let name = self.ident()?;
            let mut dims = Vec::new();
            while self.is_p("[") {
                dims.push(self.range()?);
            }
            let mut init = if self.eat_p("=") { Some(self.expr()?) } else { None };
            if let (Some(e), Some(t)) = (&init, self.decl_type.clone()) {
                init = Some(self.resolve_pattern(e.clone(), &t)?);
            }
            self.declare_var(&name, dims.len());
            v.push(Declarator { name, dims, init, line });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(";")?;
        Ok(v)
    }

    /// An interface port in an ANSI port list: `bus_if b` or `bus_if.master b` (a name that
    /// is no keyword or type, followed by a port name).
    fn is_iface_port(&self) -> bool {
        let Tok::Ident(t) = self.peek() else { return false };
        if is_keyword(t) || self.types.contains_key(t) || self.packages.contains_key(t) {
            return false;
        }
        match (self.peek_at(1), self.peek_at(2), self.peek_at(3)) {
            (Tok::Punct("."), Tok::Ident(_), Tok::Ident(_)) => true,
            (Tok::Ident(n), Tok::Punct("," | ")"), _) => !is_keyword(n),
            _ => false,
        }
    }

    /// The current token is a SystemVerilog keyword used as a Verilog-2005 identifier
    /// (`bit = 1;`, `x = int + 1`).
    fn sv_ident_here(&self) -> bool {
        // `return -x;` / `return ^v;`: unary operators after return; only an assignment makes
        // it a name
        if matches!(self.peek(), Tok::Ident(s) if s == "return") {
            return matches!(self.peek_at(1), Tok::Punct("=" | "<="));
        }
        matches!(self.peek(), Tok::Ident(s) if is_sv_only(s))
            && matches!(self.peek_at(1), Tok::Punct("=" | "<=" | ")" | "," | ";" | "." | "+" | "-" | "*" | "/" | "&" | "|" | "^" | "==" | "!=" | "?" | ":" | "}" | "]"))
    }

    fn is_decl_start(&self) -> bool {
        if self.sv_ident_here() {
            return false;
        }
        matches!(self.peek(), Tok::Ident(s) if matches!(s.as_str(),
            "input"|"output"|"inout"|"wire"|"reg"|"logic"|"bit"|"integer"|"time"|"genvar"|"int"|"byte"|"shortint"|"longint"|"real"|"realtime"|"event"
            |"tri"|"wand"|"wor"|"triand"|"trior"|"trireg"|"supply0"|"supply1"|"tri0"|"tri1"|"uwire"|"var"))
            || self.is_sv_type_start()
    }

    fn param_items(&mut self, local: bool) -> Result<Vec<ParamDecl>> {
        let pt = self.param_type()?;
        let mut v = Vec::new();
        loop {
            let line = self.line();
            let name = self.ident()?;
            self.expect_p("=")?;
            let value = self.expr()?;
            let value = match &pt.sv {
                Some(t) => self.resolve_pattern(value, t)?,
                None => value,
            };
            self.decl_type = pt.sv.clone().map(Rc::new);
            self.declare_var(&name, 0);
            v.push(ParamDecl {
                local,
                signed: pt.signed,
                range: pt.range.clone(),
                integer: pt.integer,
                real: pt.real,
                name,
                value,
                line,
            });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(";")?;
        Ok(v)
    }

    pub(crate) fn module_item(&mut self, items: &mut Vec<Item>) -> Result<()> {
        let line = self.line();
        let t = self.peek().clone();
        let kw = match &t {
            Tok::Ident(s) => s.as_str(),
            Tok::Punct(";") => {
                self.bump();
                return Ok(());
            }
            _ => return self.err(format!("unexpected {} in module", self.describe())),
        };
        match kw {
            "parameter" | "localparam" | "specparam" => {
                self.bump();
                for p in self.param_items(kw != "parameter")? {
                    items.push(Item::Param(p));
                }
            }
            "typedef" => {
                self.typedef()?;
                items.append(&mut self.pending);
            }
            "import" => self.import(false)?,
            // modports only restrict access: their names are kept, the lists skipped
            "modport" => {
                self.bump();
                loop {
                    let n = self.ident()?;
                    self.skip_parens()?;
                    self.modports.push(n);
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
            }
            "assign" => {
                self.bump();
                let strength = self.strength()?;
                let delay = if self.eat_p("#") { Some(self.delay_value()?) } else { None };
                loop {
                    let line = self.line();
                    let lhs = self.lvalue()?;
                    let lt = self.last_type.take();
                    self.expect_p("=")?;
                    let rhs = self.expr()?;
                    let rhs = match &lt {
                        Some(t) => self.resolve_pattern(rhs, t)?,
                        None => rhs,
                    };
                    items.push(Item::ContAssign { delay: delay.clone(), lhs, rhs, strength, line });
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
            }
            "initial" | "always" | "always_comb" | "always_ff" | "always_latch" | "final" => {
                self.bump();
                let kind = match kw {
                    "initial" => ProcKind::Initial,
                    "always" => ProcKind::Always,
                    "always_comb" => ProcKind::AlwaysComb,
                    "always_ff" => ProcKind::AlwaysFf,
                    "always_latch" => ProcKind::AlwaysLatch,
                    _ => ProcKind::Final,
                };
                let body = self.stmt()?;
                items.push(Item::Process { kind, body, line });
            }
            "function" | "task" => {
                let s = self.subroutine()?;
                items.push(Item::Subroutine(s));
            }
            "generate" => {
                self.bump();
                while !self.eat_kw("endgenerate") {
                    self.module_item(items)?;
                }
            }
            "for" => items.push(self.gen_for()?),
            "if" => items.push(self.gen_if()?),
            "case" => items.push(self.gen_case()?),
            "begin" => {
                self.bump();
                let name = self.opt_block_label()?;
                let mut sub = Vec::new();
                while !self.eat_kw("end") {
                    self.module_item(&mut sub)?;
                }
                self.opt_block_label()?;
                items.push(Item::GenBlock { name, items: sub, line });
            }
            "defparam" => {
                self.bump();
                loop {
                    let line = self.line();
                    let first = self.ident()?;
                    let target = match self.hier_tail(first)? {
                        Expr::Hier(p) => p,
                        Expr::Ident(n) => vec![n],
                        _ => unreachable!(),
                    };
                    self.expect_p("=")?;
                    let value = self.expr()?;
                    items.push(Item::Defparam { target, value, line });
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
            }
            "specify" => {
                // specparams and path delays are kept; timing checks are ignored
                self.bump();
                while !self.eat_kw("endspecify") {
                    if matches!(self.peek(), Tok::Eof) {
                        return self.err("unterminated specify");
                    }
                    if self.eat_kw("specparam") {
                        for p in self.param_items(true)? {
                            items.push(Item::Param(p));
                        }
                        continue;
                    }
                    // path delays (a condition in front is ignored)
                    if self.eat_kw("if") {
                        if self.is_p("(") {
                            self.skip_parens()?;
                        }
                        continue;
                    }
                    if self.eat_kw("ifnone") {
                        continue;
                    }
                    if self.is_p("(") {
                        let line = self.line();
                        if let Some(it) = self.spec_path(line)? {
                            items.push(it);
                        }
                        continue;
                    }
                    // timing checks and anything else: up to the next ';'
                    while !self.eat_p(";") {
                        if matches!(self.peek(), Tok::Eof) || self.is_kw("endspecify") {
                            break;
                        }
                        self.bump();
                    }
                }
            }
            _ if self.is_decl_start() => {
                let mut d = self.decl_head(true)?;
                d.names = self.declarators()?;
                items.append(&mut self.pending);
                items.push(Item::Decl(d));
            }
            _ if GATES.contains(&kw) => {
                self.bump();
                let kind = kw.to_string();
                let strength = self.strength()?;
                let delay = if self.eat_p("#") { Some(self.delay_value()?) } else { None };
                loop {
                    let line = self.line();
                    let inst_name = if let Tok::Ident(_) = self.peek() { Some(self.ident()?) } else { None };
                    let dims = if self.is_p("[") { Some(self.range()?) } else { None };
                    self.expect_p("(")?;
                    let mut terms = Vec::new();
                    loop {
                        terms.push(self.expr()?);
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                    self.expect_p(")")?;
                    items.push(Item::Gate { kind: kind.clone(), inst_name, dims, terms, strength, delay: delay.clone(), line });
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
            }
            _ if !is_keyword(kw) => {
                // module instantiation (the module's name is reported where it is declared)
                let module = self.ident()?;
                if self.kw_names.last().is_some_and(|(n, _)| *n == module) {
                    self.kw_names.pop();
                }
                let mut params = Vec::new();
                if self.eat_p("#") {
                    if self.eat_p("(") {
                        params = self.conn_list()?;
                    } else {
                        // #8
                        params.push(Conn::Positional(Some(self.primary()?)));
                    }
                }
                let mut insts = Vec::new();
                loop {
                    let line = self.line();
                    // an instance without a name (allowed for primitives, accepted for modules)
                    let name = if self.is_p("(") {
                        self.unnamed += 1;
                        format!("${module}${}", self.unnamed)
                    } else {
                        self.ident()?
                    };
                    let mut dims = Vec::new();
                    while self.is_p("[") {
                        dims.push(self.range()?);
                    }
                    self.expect_p("(")?;
                    let conns = self.conn_list()?;
                    insts.push(Instance { name, dims, conns, line });
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
                items.push(Item::Instances { module, params, insts, line });
            }
            _ => return self.err(format!("unsupported module item '{kw}'")),
        }
        Ok(())
    }

    /// After '(' — parses connections up to and including ')'.
    fn conn_list(&mut self) -> Result<Vec<Conn>> {
        let mut v = Vec::new();
        if self.eat_p(")") {
            return Ok(v);
        }
        loop {
            if self.eat_p(".") {
                if self.eat_p("*") {
                    v.push(Conn::Wildcard);
                } else {
                    let name = self.ident()?;
                    if self.eat_p("(") {
                        let e = if self.is_p(")") { None } else { Some(self.expr()?) };
                        self.expect_p(")")?;
                        v.push(Conn::Named(name, e));
                    } else {
                        // SV .name implicit
                        v.push(Conn::Named(name.clone(), Some(Expr::Ident(name))));
                    }
                }
            } else if self.is_p(",") || self.is_p(")") {
                v.push(Conn::Positional(None));
            } else {
                v.push(Conn::Positional(Some(self.expr()?)));
            }
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        Ok(v)
    }

    fn gen_body(&mut self) -> Result<(Option<String>, Vec<Item>)> {
        let mut items = Vec::new();
        if self.eat_kw("begin") {
            let name = self.opt_block_label()?;
            while !self.eat_kw("end") {
                self.module_item(&mut items)?;
            }
            self.opt_block_label()?;
            Ok((name, items))
        } else {
            self.module_item(&mut items)?;
            Ok((None, items))
        }
    }

    fn gen_for(&mut self) -> Result<Item> {
        let line = self.line();
        self.expect_kw("for")?;
        self.expect_p("(")?;
        self.eat_kw("genvar");
        let init_var = self.ident()?;
        self.expect_p("=")?;
        let init = self.expr()?;
        self.expect_p(";")?;
        let cond = self.expr()?;
        self.expect_p(";")?;
        let step_var = self.ident()?;
        let step = self.assign_rhs_for_step(&step_var)?;
        self.expect_p(")")?;
        let (name, items) = self.gen_body()?;
        Ok(Item::GenFor { init_var, init, cond, step_var, step, name, items, line })
    }

    /// parses `= expr`, `++`, `--`, `+= expr` etc., returning the new value expression
    fn assign_rhs_for_step(&mut self, var: &str) -> Result<Expr> {
        let v = Box::new(Expr::Ident(var.to_string()));
        if self.eat_p("=") {
            return self.expr();
        }
        if self.eat_p("++") {
            return Ok(Expr::Binary(BinOp::Add, v, Box::new(Expr::Num(dec(1)))));
        }
        if self.eat_p("--") {
            return Ok(Expr::Binary(BinOp::Sub, v, Box::new(Expr::Num(dec(1)))));
        }
        let op = match self.peek() {
            Tok::Punct("+=") => BinOp::Add,
            Tok::Punct("-=") => BinOp::Sub,
            Tok::Punct("*=") => BinOp::Mul,
            Tok::Punct("/=") => BinOp::Div,
            Tok::Punct("%=") => BinOp::Mod,
            Tok::Punct("&=") => BinOp::And,
            Tok::Punct("|=") => BinOp::Or,
            Tok::Punct("^=") => BinOp::Xor,
            Tok::Punct("<<=") => BinOp::Shl,
            Tok::Punct(">>=") => BinOp::Shr,
            Tok::Punct("<<<=") => BinOp::AShl,
            Tok::Punct(">>>=") => BinOp::AShr,
            _ => return self.err("expected assignment"),
        };
        self.bump();
        let rhs = self.expr()?;
        Ok(Expr::Binary(op, v, Box::new(rhs)))
    }

    fn gen_if(&mut self) -> Result<Item> {
        let line = self.line();
        self.expect_kw("if")?;
        self.expect_p("(")?;
        let cond = self.expr()?;
        self.expect_p(")")?;
        let (then_name, then_items) = self.gen_body()?;
        let (else_name, else_items) = if self.eat_kw("else") {
            if self.is_kw("if") {
                let i = self.gen_if()?;
                (None, vec![i])
            } else {
                self.gen_body()?
            }
        } else {
            (None, Vec::new())
        };
        Ok(Item::GenIf { cond, then_items, then_name, else_items, else_name, line })
    }

    fn gen_case(&mut self) -> Result<Item> {
        let line = self.line();
        self.expect_kw("case")?;
        self.expect_p("(")?;
        let expr = self.expr()?;
        self.expect_p(")")?;
        let mut arms = Vec::new();
        while !self.eat_kw("endcase") {
            let mut labels = Vec::new();
            if self.eat_kw("default") {
                self.eat_p(":");
            } else {
                loop {
                    labels.push(self.expr()?);
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(":")?;
            }
            let (name, items) = self.gen_body()?;
            arms.push((labels, name, items));
        }
        Ok(Item::GenCase { expr, arms, line })
    }

    fn subroutine(&mut self) -> Result<SubroutineDecl> {
        let line = self.line();
        let is_task = self.is_kw("task");
        self.bump();
        let automatic = self.eat_kw("automatic");
        self.eat_kw("static");
        let mut signed = false;
        let mut range = None;
        let mut ret_kind = NetKind::Reg;
        let mut ret_type = None;
        if !is_task && self.is_sv_type_start() {
            let t = self.data_type()?.unwrap();
            let (k, sg, r, packed) = t.decl_parts();
            if !packed.is_empty() {
                range = Some((crate::svtypes::minus_one(t.width()), Expr::Num(dec(0))));
            } else {
                range = r;
            }
            signed = sg;
            ret_kind = match k {
                NetKind::Int(_) | NetKind::Integer if range.is_none() => NetKind::Integer,
                _ => NetKind::Reg,
            };
            if ret_kind == NetKind::Integer {
                range = None;
            }
            ret_type = Some(Rc::new(t));
        } else if !is_task {
            if self.eat_kw("void") {
                ret_kind = NetKind::Int(0);
            }
            loop {
                if self.eat_kw("signed") {
                    signed = true;
                } else if self.eat_kw("unsigned") || self.eat_kw("reg") || self.eat_kw("logic") || self.eat_kw("bit") {
                } else if self.eat_kw("integer") || self.eat_kw("int") {
                    ret_kind = NetKind::Integer;
                    signed = true;
                } else if self.eat_kw("real") || self.eat_kw("realtime") {
                    ret_kind = NetKind::Real;
                } else {
                    break;
                }
            }
            if self.is_p("[") {
                range = Some(self.range()?);
            }
        }
        let name = self.ident()?;
        // the function's own name is its result variable
        self.ret_type = ret_type.clone();
        self.decl_type = ret_type;
        self.declare_var(&name, 0);
        let mut decls = Vec::new();
        if self.eat_p("(") {
            if !self.is_p(")") {
                let mut cur: Option<Decl> = None;
                loop {
                    if matches!(self.peek(), Tok::Ident(s) if s == "input" || s == "output" || s == "inout")
                        || cur.is_none()
                        || self.is_decl_start()
                    {
                        let mut d = self.decl_head(true)?;
                        if d.dir.is_none() {
                            d.dir = Some(Dir::Input);
                        }
                        if !d.explicit_kind {
                            d.kind = NetKind::Reg;
                        }
                        cur = Some(d);
                    }
                    let dl = self.line();
                    let n = self.ident()?;
                    self.declare_var(&n, 0);
                    let mut d = cur.clone().unwrap();
                    d.names = vec![Declarator { name: n, dims: vec![], init: None, line: dl }];
                    decls.push(d);
                    if !self.eat_p(",") {
                        break;
                    }
                }
            }
            self.expect_p(")")?;
        }
        self.expect_p(";")?;
        let end = if is_task { "endtask" } else { "endfunction" };
        let mut stmts = Vec::new();
        loop {
            if self.eat_kw(end) {
                break;
            }
            if self.is_decl_start() {
                let mut d = self.decl_head(true)?;
                if !d.explicit_kind {
                    d.kind = NetKind::Reg;
                }
                d.names = self.declarators()?;
                decls.push(d);
            } else if self.is_kw("parameter") || self.is_kw("localparam") {
                return self.err("parameters inside functions are not supported");
            } else {
                stmts.push(self.stmt()?);
            }
        }
        self.opt_block_label()?;
        let body = if stmts.len() == 1 {
            stmts.pop().unwrap()
        } else {
            Stmt::Block { name: None, decls: vec![], stmts, fork: None }
        };
        let decls = merge_port_types(decls);
        self.ret_type = None;
        Ok(SubroutineDecl { name, is_task, automatic, signed, range, ret_kind, decls, body, line })
    }

    // ---------- statements ----------
    /// `( strength0, strength1 )` or a charge strength `(small)`: skipped (no strength model)
    /// `(strength0, strength1)` (either order, or one of them): levels 0 = highz .. 7 = supply;
    /// a missing one is strong. Charge strengths (small, medium, large) give None.
    fn strength(&mut self) -> Result<Option<(u8, u8)>> {
        if !(self.is_p("(") && matches!(self.peek_at(1), Tok::Ident(s) if STRENGTHS.contains(&s.as_str()))) {
            return Ok(None);
        }
        self.bump();
        let (mut s0, mut s1, mut drive) = (6u8, 6u8, false);
        while !self.eat_p(")") {
            match self.bump() {
                Tok::Ident(s) => {
                    let lvl = match &s[..s.len() - 1] {
                        "supply" => 7,
                        "strong" => 6,
                        "pull" => 5,
                        "weak" => 3,
                        "highz" => 0,
                        _ => continue,
                    };
                    drive = true;
                    if s.ends_with('0') {
                        s0 = lvl;
                    } else {
                        s1 = lvl;
                    }
                }
                Tok::Punct(",") => {}
                Tok::Eof => return self.err("unterminated strength"),
                _ => {}
            }
        }
        Ok(drive.then_some((s0, s1)))
    }



    fn delay_value(&mut self) -> Result<Expr> {
        if self.eat_p("(") {
            let mtm = |p: &mut Parser| -> Result<Expr> {
                let e = p.expr()?;
                // min:typ:max -> take typ
                if p.eat_p(":") {
                    let t = p.expr()?;
                    p.expect_p(":")?;
                    p.expr()?;
                    return Ok(t);
                }
                Ok(e)
            };
            let e = mtm(self)?;
            // (rise, fall[, turn-off]): kept as a pseudo call; procedural delays use the first
            let mut v = vec![e];
            while self.eat_p(",") {
                v.push(mtm(self)?);
            }
            self.expect_p(")")?;
            Ok(if v.len() == 1 { v.pop().unwrap() } else { Expr::SysCall("$__delays".into(), v) })
        } else {
            match self.bump() {
                Tok::Num(n) => Ok(Expr::Num(n)),
                Tok::Real(r) => Ok(Expr::Real(r)),
                Tok::Ident(s) => Ok(Expr::Ident(s)),
                _ => self.err("bad delay value"),
            }
        }
    }

    fn event_ctl(&mut self) -> Result<EventCtl> {
        // after '@'
        if self.eat_p("*") {
            return Ok(EventCtl::Star);
        }
        if !self.eat_p("(") {
            // @name or @a.b.name
            let id = self.ident()?;
            let e = self.hier_tail(id)?;
            return Ok(EventCtl::List(vec![(Edge::Any, e)]));
        }
        if self.eat_p("*") {
            self.expect_p(")")?;
            return Ok(EventCtl::Star);
        }
        let mut v = Vec::new();
        loop {
            let edge = if self.eat_kw("posedge") {
                Edge::Pos
            } else if self.eat_kw("negedge") {
                Edge::Neg
            } else {
                Edge::Any
            };
            let e = self.expr()?;
            v.push((edge, e));
            if !(self.eat_kw("or") || self.eat_p(",")) {
                break;
            }
        }
        self.expect_p(")")?;
        Ok(EventCtl::List(v))
    }

    /// Statements up to `end` (or join_any / join_none for a fork). Returns the terminator.
    fn block_body(&mut self, end: &str) -> Result<(Vec<Decl>, Vec<Stmt>, String)> {
        let mut decls = Vec::new();
        let mut stmts = Vec::new();
        let term;
        loop {
            if self.is_kw(end) || (end == "join" && (self.is_kw("join_any") || self.is_kw("join_none"))) {
                term = match self.bump() {
                    Tok::Ident(s) => s,
                    _ => unreachable!(),
                };
                break;
            }
            if matches!(self.peek(), Tok::Eof) {
                return self.err("unexpected end of file in block");
            }
            if self.is_decl_start() {
                let mut d = self.decl_head(false)?;
                if !d.explicit_kind {
                    d.kind = NetKind::Reg;
                }
                d.names = self.declarators()?;
                decls.push(d);
            } else if self.is_kw("parameter") || self.is_kw("localparam") {
                return self.err("parameters in blocks are not supported");
            } else {
                stmts.push(self.stmt()?);
            }
        }
        Ok((decls, stmts, term))
    }

    pub fn stmt(&mut self) -> Result<Stmt> {
        let line = self.line();
        match self.peek().clone() {
            Tok::Punct(";") => {
                self.bump();
                Ok(Stmt::Null)
            }
            Tok::Punct("#") => {
                self.bump();
                let d = self.delay_value()?;
                let body = self.stmt()?;
                Ok(Stmt::Timed { timing: Timing::Delay(d), body: Box::new(body), line })
            }
            Tok::Punct("@") => {
                self.bump();
                let ec = self.event_ctl()?;
                let body = self.stmt()?;
                Ok(Stmt::Timed { timing: Timing::Event(ec), body: Box::new(body), line })
            }
            Tok::Punct("->") => {
                self.bump();
                let name = self.dotted_name()?;
                self.expect_p(";")?;
                Ok(Stmt::Trigger { name, line })
            }
            Tok::SysIdent(name) => {
                self.bump();
                let mut args = Vec::new();
                if self.eat_p("(") {
                    if !self.is_p(")") {
                        loop {
                            if self.is_p(",") || self.is_p(")") {
                                args.push(None);
                            } else {
                                args.push(Some(self.expr()?));
                            }
                            if !self.eat_p(",") {
                                break;
                            }
                        }
                    }
                    self.expect_p(")")?;
                }
                self.expect_p(";")?;
                Ok(Stmt::SysTask { name, args, line })
            }
            Tok::Punct("{") => self.assign_stmt(),
            Tok::Ident(_) if self.sv_ident_here() => self.assign_stmt(),
            Tok::Ident(kw) => match kw.as_str() {
                "begin" | "fork" => {
                    self.bump();
                    let fork = kw == "fork";
                    let name = self.opt_block_label()?;
                    let (decls, stmts, term) = self.block_body(if fork { "join" } else { "end" })?;
                    self.opt_block_label()?;
                    let fork = match term.as_str() {
                        "join" => Some(Join::All),
                        "join_any" => Some(Join::Any),
                        "join_none" => Some(Join::None),
                        _ => None,
                    };
                    Ok(Stmt::Block { name, decls, stmts, fork })
                }
                "if" => {
                    self.bump();
                    self.expect_p("(")?;
                    let cond = self.expr()?;
                    self.expect_p(")")?;
                    let then_s = Box::new(self.stmt()?);
                    let else_s = if self.eat_kw("else") { Some(Box::new(self.stmt()?)) } else { None };
                    Ok(Stmt::If { cond, then_s, else_s, line })
                }
                "unique" | "priority" | "unique0" => {
                    self.bump();
                    self.stmt()
                }
                "case" | "casez" | "casex" => {
                    self.bump();
                    let kind = match kw.as_str() {
                        "case" => CaseKind::Case,
                        "casez" => CaseKind::CaseZ,
                        _ => CaseKind::CaseX,
                    };
                    self.expect_p("(")?;
                    let expr = self.expr()?;
                    self.expect_p(")")?;
                    let mut arms = Vec::new();
                    let mut default = None;
                    while !self.eat_kw("endcase") {
                        if self.eat_kw("default") {
                            self.eat_p(":");
                            default = Some(Box::new(self.stmt()?));
                            continue;
                        }
                        let mut labels = Vec::new();
                        loop {
                            labels.push(self.expr()?);
                            if !self.eat_p(",") {
                                break;
                            }
                        }
                        self.expect_p(":")?;
                        let s = self.stmt()?;
                        arms.push((labels, s));
                    }
                    Ok(Stmt::Case { kind, expr, arms, default, line })
                }
                "for" => {
                    self.bump();
                    self.expect_p("(")?;
                    let init = if self.is_decl_start() {
                        // SV: for (int i = 0; ...) -- declare as block-local variable
                        let mut d = self.decl_head(false)?;
                        let dl = self.line();
                        let name = self.ident()?;
                        self.expect_p("=")?;
                        let v = self.expr()?;
                        self.expect_p(";")?;
                        d.names = vec![Declarator { name: name.clone(), dims: vec![], init: None, line: dl }];
                        let cond = self.expr()?;
                        self.expect_p(";")?;
                        let step = self.simple_assign_noterm()?;
                        self.expect_p(")")?;
                        let body = self.stmt()?;
                        let f = Stmt::For {
                            init: Box::new(Stmt::Assign {
                                lhs: Expr::Ident(name),
                                rhs: v,
                                nonblocking: false,
                                timing: None,
                                line: dl,
                            }),
                            cond,
                            step: Box::new(step),
                            body: Box::new(body),
                            line,
                        };
                        return Ok(Stmt::Block { name: None, decls: vec![d], stmts: vec![f], fork: None });
                    } else {
                        self.simple_assign_noterm()?
                    };
                    self.expect_p(";")?;
                    let cond = self.expr()?;
                    self.expect_p(";")?;
                    let step = self.simple_assign_noterm()?;
                    self.expect_p(")")?;
                    let body = self.stmt()?;
                    Ok(Stmt::For { init: Box::new(init), cond, step: Box::new(step), body: Box::new(body), line })
                }
                "while" => {
                    self.bump();
                    self.expect_p("(")?;
                    let cond = self.expr()?;
                    self.expect_p(")")?;
                    let body = Box::new(self.stmt()?);
                    Ok(Stmt::While { cond, body, line })
                }
                "do" => {
                    self.bump();
                    let body = Box::new(self.stmt()?);
                    self.expect_kw("while")?;
                    self.expect_p("(")?;
                    let cond = self.expr()?;
                    self.expect_p(")")?;
                    self.expect_p(";")?;
                    Ok(Stmt::DoWhile { cond, body, line })
                }
                "repeat" => {
                    self.bump();
                    self.expect_p("(")?;
                    let count = self.expr()?;
                    self.expect_p(")")?;
                    let body = Box::new(self.stmt()?);
                    Ok(Stmt::Repeat { count, body, line })
                }
                "forever" => {
                    self.bump();
                    let body = Box::new(self.stmt()?);
                    Ok(Stmt::Forever { body, line })
                }
                "wait" => {
                    self.bump();
                    self.expect_p("(")?;
                    let cond = self.expr()?;
                    self.expect_p(")")?;
                    let body = Box::new(self.stmt()?);
                    Ok(Stmt::Wait { cond, body, line })
                }
                // force/release and the procedural continuous assignment assign/deassign
                "force" | "release" | "assign" | "deassign" => {
                    self.bump();
                    let lhs = self.lvalue()?;
                    let rhs = if kw == "force" || kw == "assign" {
                        self.expect_p("=")?;
                        Some(self.expr()?)
                    } else {
                        None
                    };
                    self.expect_p(";")?;
                    Ok(Stmt::Force { lhs, rhs, line })
                }
                "disable" => {
                    self.bump();
                    let name = if self.eat_kw("fork") { "fork".to_string() } else { self.dotted_name()? };
                    self.expect_p(";")?;
                    Ok(Stmt::Disable { name, line })
                }
                "return" => {
                    self.bump();
                    let mut value = if self.is_p(";") { None } else { Some(self.expr()?) };
                    if let (Some(v), Some(t)) = (&value, self.ret_type.clone()) {
                        value = Some(self.resolve_pattern(v.clone(), &t)?);
                    }
                    self.expect_p(";")?;
                    Ok(Stmt::Return { value, line })
                }
                "break" => {
                    self.bump();
                    self.expect_p(";")?;
                    Ok(Stmt::Break { line })
                }
                "continue" => {
                    self.bump();
                    self.expect_p(";")?;
                    Ok(Stmt::Continue { line })
                }
                _ if is_keyword(&kw) => self.err(format!("unexpected '{kw}' in statement")),
                _ => {
                    // task call or assignment
                    if matches!(self.peek_at(1), Tok::Punct("(")) || matches!(self.peek_at(1), Tok::Punct(";")) {
                        let name = self.ident()?;
                        let mut args = Vec::new();
                        if self.eat_p("(") {
                            if !self.is_p(")") {
                                loop {
                                    args.push(self.expr()?);
                                    if !self.eat_p(",") {
                                        break;
                                    }
                                }
                            }
                            self.expect_p(")")?;
                        }
                        self.expect_p(";")?;
                        return Ok(Stmt::TaskCall { name, args, line });
                    }
                    self.assign_stmt()
                }
            },
            _ => self.err(format!("unexpected {} at start of statement", self.describe())),
        }
    }

    fn assign_stmt(&mut self) -> Result<Stmt> {
        // hierarchical task call: inst.task; or inst.task(args);
        if let Tok::Ident(_) = self.peek() {
            let save = self.pos;
            let line = self.line();
            let first = self.ident()?;
            if let Expr::Hier(parts) = self.hier_tail(first)? {
                if self.is_p(";") || self.is_p("(") {
                    let mut args = Vec::new();
                    if self.eat_p("(") {
                        if !self.is_p(")") {
                            loop {
                                args.push(self.expr()?);
                                if !self.eat_p(",") {
                                    break;
                                }
                            }
                        }
                        self.expect_p(")")?;
                    }
                    self.expect_p(";")?;
                    return Ok(Stmt::TaskCall { name: parts.join("."), args, line });
                }
            }
            self.pos = save;
        }
        let s = self.simple_assign_noterm()?;
        self.expect_p(";")?;
        Ok(s)
    }

    /// lvalue (= | <= | op=) [timing] expr, or lvalue++ / lvalue--
    fn simple_assign_noterm(&mut self) -> Result<Stmt> {
        let line = self.line();
        let lhs = self.lvalue()?;
        let lt = self.last_type.take();
        let nonblocking = if self.eat_p("=") {
            false
        } else if self.eat_p("<=") {
            true
        } else {
            // compound / inc-dec
            let rhs = match self.peek() {
                Tok::Punct("++") => {
                    self.bump();
                    Expr::Binary(BinOp::Add, Box::new(lhs.clone()), Box::new(Expr::Num(dec(1))))
                }
                Tok::Punct("--") => {
                    self.bump();
                    Expr::Binary(BinOp::Sub, Box::new(lhs.clone()), Box::new(Expr::Num(dec(1))))
                }
                _ => {
                    let op = match self.peek() {
                        Tok::Punct("+=") => BinOp::Add,
                        Tok::Punct("-=") => BinOp::Sub,
                        Tok::Punct("*=") => BinOp::Mul,
                        Tok::Punct("/=") => BinOp::Div,
                        Tok::Punct("%=") => BinOp::Mod,
                        Tok::Punct("&=") => BinOp::And,
                        Tok::Punct("|=") => BinOp::Or,
                        Tok::Punct("^=") => BinOp::Xor,
                        Tok::Punct("<<=") => BinOp::Shl,
                        Tok::Punct(">>=") => BinOp::Shr,
                        Tok::Punct("<<<=") => BinOp::AShl,
                        Tok::Punct(">>>=") => BinOp::AShr,
                        _ => return self.err(format!("expected '=' or '<=', found {}", self.describe())),
                    };
                    self.bump();
                    let r = self.expr()?;
                    Expr::Binary(op, Box::new(lhs.clone()), Box::new(r))
                }
            };
            return Ok(Stmt::Assign { lhs, rhs, nonblocking: false, timing: None, line });
        };
        let timing = if self.eat_p("#") {
            Some(Timing::Delay(self.delay_value()?))
        } else if self.eat_p("@") {
            Some(Timing::Event(self.event_ctl()?))
        } else if self.eat_kw("repeat") {
            self.expect_p("(")?;
            let n = self.expr()?;
            self.expect_p(")")?;
            self.expect_p("@")?;
            Some(Timing::Repeat(n, self.event_ctl()?))
        } else {
            None
        };
        let rhs = self.expr()?;
        let rhs = match &lt {
            Some(t) => self.resolve_pattern(rhs, t)?,
            None => rhs,
        };
        Ok(Stmt::Assign { lhs, rhs, nonblocking, timing, line })
    }

    fn lvalue(&mut self) -> Result<Expr> {
        if self.eat_p("{") {
            let mut v = Vec::new();
            loop {
                v.push(self.lvalue()?);
                if !self.eat_p(",") {
                    break;
                }
            }
            self.expect_p("}")?;
            return Ok(Expr::Concat(v));
        }
        let name = self.ident()?;
        self.last_type = None;
        if let Some((t, unpacked)) = self.vars.get(&name).cloned() {
            if t.has_struct() {
                let e = self.struct_access(name, true)?;
                return self.selects(e);
            }
            if unpacked == 0 && !self.is_p("[") {
                self.last_type = Some((*t).clone());
            }
        }
        let base = self.hier_tail(name)?;
        self.selects(base)
    }

    fn hier_tail(&mut self, name: String) -> Result<Expr> {
        let mut parts = vec![name];
        loop {
            let seg = self.hier_index(parts.last().unwrap());
            if let Some(s) = seg {
                *parts.last_mut().unwrap() = s;
            }
            if self.is_p(".") && matches!(self.peek_at(1), Tok::Ident(_)) {
                self.eat_p(".");
                parts.push(self.ident()?);
            } else {
                break;
            }
        }
        if parts.len() == 1 {
            return Ok(Expr::Ident(parts.pop().unwrap()));
        }
        Ok(Expr::Hier(parts))
    }

    /// `name[N].` in a hierarchical name (an instance-array element or generate block):
    /// returns `name[N]` and consumes `[N]` when a literal index is followed by `.ident`.
    fn hier_index(&mut self, name: &str) -> Option<String> {
        if !self.is_p("[") {
            return None;
        }
        let save = self.pos;
        self.eat_p("[");
        let v = match self.expr() {
            Ok(Expr::Num(n)) if !n.digits.contains(['x', 'z', 'X', 'Z', '?']) => {
                i64::from_str_radix(&n.digits.replace('_', ""), n.base as u32).ok()
            }
            Ok(Expr::Unary(UnOp::Neg, b)) => match *b {
                Expr::Num(n) => i64::from_str_radix(&n.digits.replace('_', ""), n.base as u32).ok().map(|v| -v),
                _ => None,
            },
            _ => None,
        };
        if let Some(v) = v {
            if self.eat_p("]") && self.is_p(".") && matches!(self.peek_at(1), Tok::Ident(_)) {
                return Some(format!("{name}[{v}]"));
            }
        }
        // a computed index (genvar expression): kept aside, resolved during elaboration
        self.pos = save;
        self.eat_p("[");
        if let Ok(e) = self.expr() {
            if self.eat_p("]") && self.is_p(".") && matches!(self.peek_at(1), Tok::Ident(_)) {
                let n = HIER_INDEX.with(|h| {
                    let mut h = h.borrow_mut();
                    h.push(e);
                    h.len() - 1
                });
                return Some(format!("{name}[@{n}]"));
            }
        }
        self.pos = save;
        None
    }

    fn selects(&mut self, mut e: Expr) -> Result<Expr> {
        while self.eat_p("[") {
            let a = self.expr()?;
            if self.eat_p(":") {
                let b = self.expr()?;
                self.expect_p("]")?;
                e = Expr::Range(Box::new(e), Box::new(a), Box::new(b));
            } else if self.eat_p("+:") {
                let b = self.expr()?;
                self.expect_p("]")?;
                e = Expr::IndexedRange(Box::new(e), Box::new(a), Box::new(b), true);
            } else if self.eat_p("-:") {
                let b = self.expr()?;
                self.expect_p("]")?;
                e = Expr::IndexedRange(Box::new(e), Box::new(a), Box::new(b), false);
            } else {
                self.expect_p("]")?;
                e = Expr::Index(Box::new(e), Box::new(a));
            }
        }
        Ok(e)
    }

    // ---------- expressions ----------
    pub fn expr(&mut self) -> Result<Expr> {
        let c = self.binary(1)?;
        if self.eat_p("?") {
            let t = self.expr()?;
            self.expect_p(":")?;
            let f = self.expr()?;
            return Ok(Expr::Cond(Box::new(c), Box::new(t), Box::new(f)));
        }
        Ok(c)
    }

    fn binop(&self) -> Option<(BinOp, u8)> {
        let p = match self.peek() {
            Tok::Punct(p) => *p,
            _ => return None,
        };
        Some(match p {
            "||" => (BinOp::LOr, 1),
            "&&" => (BinOp::LAnd, 2),
            "|" => (BinOp::Or, 3),
            "^" => (BinOp::Xor, 4),
            "~^" | "^~" => (BinOp::Xnor, 4),
            "&" => (BinOp::And, 5),
            "==" => (BinOp::Eq, 6),
            "!=" => (BinOp::Ne, 6),
            "===" => (BinOp::CaseEq, 6),
            "!==" => (BinOp::CaseNe, 6),
            "==?" => (BinOp::WildEq, 6),
            "!=?" => (BinOp::WildNe, 6),
            "<" => (BinOp::Lt, 7),
            "<=" => (BinOp::Le, 7),
            ">" => (BinOp::Gt, 7),
            ">=" => (BinOp::Ge, 7),
            "<<" => (BinOp::Shl, 8),
            ">>" => (BinOp::Shr, 8),
            "<<<" => (BinOp::AShl, 8),
            ">>>" => (BinOp::AShr, 8),
            "+" => (BinOp::Add, 9),
            "-" => (BinOp::Sub, 9),
            "*" => (BinOp::Mul, 10),
            "/" => (BinOp::Div, 10),
            "%" => (BinOp::Mod, 10),
            "**" => (BinOp::Pow, 11),
            _ => return None,
        })
    }

    fn binary(&mut self, min_prec: u8) -> Result<Expr> {
        let mut lhs = self.unary()?;
        while let Some((op, prec)) = self.binop() {
            if prec < min_prec {
                break;
            }
            self.bump();
            // ** is right-assoc
            let next = if op == BinOp::Pow { prec } else { prec + 1 };
            let rhs = self.binary(next)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr> {
        let op = match self.peek() {
            Tok::Punct("+") => Some(UnOp::Plus),
            Tok::Punct("-") => Some(UnOp::Neg),
            Tok::Punct("~") => Some(UnOp::Not),
            Tok::Punct("!") => Some(UnOp::LNot),
            Tok::Punct("&") => Some(UnOp::RedAnd),
            Tok::Punct("|") => Some(UnOp::RedOr),
            Tok::Punct("^") => Some(UnOp::RedXor),
            Tok::Punct("~&") => Some(UnOp::RedNand),
            Tok::Punct("~|") => Some(UnOp::RedNor),
            Tok::Punct("~^") | Tok::Punct("^~") => Some(UnOp::RedXnor),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let e = self.unary()?;
            return Ok(Expr::Unary(op, Box::new(e)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr> {
        // pkg::name
        self.pkg_scope()?;
        let t = self.bump();
        let e = match t {
            // size cast: 8'(e)
            Tok::Num(n) if self.is_p("'") && matches!(self.peek_at(1), Tok::Punct("(")) => {
                self.bump();
                self.bump();
                let e = self.expr()?;
                self.expect_p(")")?;
                crate::svtypes::cast(Expr::Num(n), e, 2)
            }
            Tok::Num(n) => Expr::Num(n),
            // assignment pattern '{...}
            Tok::Punct("'") if self.is_p("{") => self.pattern()?,
            Tok::Real(r) => Expr::Real(r),
            Tok::Str(s) => Expr::Str(s),
            Tok::Punct("(") => {
                let e = self.expr()?;
                // min:typ:max
                let e = if self.eat_p(":") {
                    let t = self.expr()?;
                    self.expect_p(":")?;
                    self.expr()?;
                    t
                } else {
                    e
                };
                self.expect_p(")")?;
                e
            }
            Tok::Punct("{") => {
                let first = self.expr()?;
                if self.eat_p("{") {
                    // replication
                    let mut v = Vec::new();
                    loop {
                        v.push(self.expr()?);
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                    self.expect_p("}")?;
                    self.expect_p("}")?;
                    Expr::Repl(Box::new(first), v)
                } else {
                    let mut v = vec![first];
                    while self.eat_p(",") {
                        v.push(self.expr()?);
                    }
                    self.expect_p("}")?;
                    Expr::Concat(v)
                }
            }
            Tok::SysIdent(name) if name == "$bits" && self.is_p("(") && (matches!(self.peek_at(1), Tok::Ident(s) if self.types.contains_key(s) || s == "struct" || s == "enum") || self.is_pkg_type_at(1)) => {
                self.bump();
                let w = self.type_width_here()?;
                self.expect_p(")")?;
                match w {
                    Some(w) => w,
                    None => return self.err("expected a type"),
                }
            }
            Tok::SysIdent(name) => {
                let mut args = Vec::new();
                let mut type_bits = None;
                if self.eat_p("(") {
                    if !self.is_p(")") {
                        loop {
                            // a type as the argument: $bits(integer)
                            let tw = match self.peek() {
                                Tok::Ident(t) if matches!(self.peek_at(1), Tok::Punct(")" | ",")) => match t.as_str() {
                                    "integer" | "int" => Some(32),
                                    "time" | "real" | "realtime" | "longint" => Some(64),
                                    "shortint" => Some(16),
                                    "byte" => Some(8),
                                    "bit" | "logic" | "reg" => Some(1),
                                    _ => None,
                                },
                                _ => None,
                            };
                            match tw {
                                Some(w) if name == "$bits" => {
                                    self.bump();
                                    type_bits = Some(w);
                                    args.push(Expr::Num(dec(w)));
                                }
                                _ => args.push(self.expr()?),
                            }
                            if !self.eat_p(",") {
                                break;
                            }
                        }
                    }
                    self.expect_p(")")?;
                }
                match type_bits {
                    Some(w) => Expr::Num(dec(w)),
                    None => Expr::SysCall(name, args),
                }
            }
            // casts: T'(e), int'(e), signed'(e)
            Tok::Ident(name) if self.is_p("'") && matches!(self.peek_at(1), Tok::Punct("(")) && (self.types.contains_key(&name) || matches!(name.as_str(), "int" | "integer" | "shortint" | "byte" | "longint" | "bit" | "logic" | "reg" | "signed" | "unsigned")) => {
                if name == "signed" || name == "unsigned" {
                    self.bump();
                    self.bump();
                    let e = self.expr()?;
                    self.expect_p(")")?;
                    return Ok(Expr::SysCall(format!("${name}"), vec![e]));
                }
                self.pos -= 1;
                let t = self.data_type()?.unwrap();
                return self.cast_to(&t);
            }
            Tok::Ident(name) if self.vars.get(&name).is_some_and(|v| v.0.has_struct()) && !self.is_p("(") => {
                let e = self.struct_access(name, false)?;
                return self.selects(e);
            }
            Tok::Ident(name) if !is_keyword(&name) || (is_sv_only(&name) && !self.is_p("'")) => {
                if self.is_p("(") {
                    self.bump();
                    let mut args = Vec::new();
                    if !self.is_p(")") {
                        loop {
                            args.push(self.expr()?);
                            if !self.eat_p(",") {
                                break;
                            }
                        }
                    }
                    self.expect_p(")")?;
                    return Ok(Expr::Call(name, args));
                }
                let base = self.hier_tail(name)?;
                if let (Expr::Hier(parts), true) = (&base, self.is_p("(")) {
                    // hierarchical function call: inst.f(args)
                    let name = parts.join(".");
                    self.bump();
                    let mut args = Vec::new();
                    if !self.is_p(")") {
                        loop {
                            args.push(self.expr()?);
                            if !self.eat_p(",") {
                                break;
                            }
                        }
                    }
                    self.expect_p(")")?;
                    return Ok(Expr::Call(name, args));
                }
                return self.selects(base);
            }
            _ => {
                self.pos -= 1;
                return self.err(format!("unexpected {} in expression", self.describe()));
            }
        };
        // selects after parenthesised/concat expressions are not legal Verilog, but allow on concat
        Ok(e)
    }
}

/// Non-ANSI subroutine ports typed by a second declaration (`input x; integer x;`): the
/// port takes the type, and the type-only declaration is dropped.
fn merge_port_types(decls: Vec<Decl>) -> Vec<Decl> {
    let typed: Vec<&Decl> = decls.iter().filter(|d| d.dir.is_none()).collect();
    let find = |n: &str| typed.iter().find(|d| d.names.iter().any(|x| x.name == n)).copied();
    let port_names: Vec<String> =
        decls.iter().filter(|d| d.dir.is_some()).flat_map(|d| d.names.iter().map(|x| x.name.clone())).collect();
    if !port_names.iter().any(|n| find(n).is_some()) {
        return decls;
    }
    let mut out = Vec::new();
    for d in &decls {
        if d.dir.is_some() {
            for x in &d.names {
                let mut nd = d.clone();
                nd.names = vec![x.clone()];
                if let Some(t) = find(&x.name) {
                    nd.kind = t.kind.clone();
                    nd.net = t.net;
                    nd.explicit_kind = true;
                    nd.signed |= t.signed;
                    if t.range.is_some() {
                        nd.range = t.range.clone();
                    }
                }
                out.push(nd);
            }
        } else {
            let mut nd = d.clone();
            nd.names.retain(|x| !port_names.contains(&x.name));
            if !nd.names.is_empty() {
                out.push(nd);
            }
        }
    }
    out
}

thread_local! {
    /// Index expressions of hierarchical-name segments that are not literals (`blk[i].x`): the
    /// segment is stored as `blk[@N]` and resolved by the compiler in the referencing scope.
    static HIER_INDEX: std::cell::RefCell<Vec<Expr>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The index expression of a `name[@N]` hierarchical-name segment.
pub fn hier_index_expr(n: usize) -> Option<Expr> {
    HIER_INDEX.with(|h| h.borrow().get(n).cloned())
}

pub fn dec(v: u64) -> Number {
    Number { size: None, signed: true, base: 10, based: false, digits: v.to_string(), fill: false }
}

/// The name an item declares (for file-scope items a module may redeclare).
pub(crate) fn item_name(it: &Item) -> Option<&str> {
    match it {
        Item::Param(p) => Some(&p.name),
        Item::Subroutine(f) => Some(&f.name),
        Item::Decl(d) if d.names.len() == 1 => Some(&d.names[0].name),
        _ => None,
    }
}

fn declared_names(items: &[Item]) -> std::collections::HashSet<String> {
    let mut v = std::collections::HashSet::new();
    for it in items {
        match it {
            Item::Decl(d) => v.extend(d.names.iter().map(|n| n.name.clone())),
            Item::Param(p) => {
                v.insert(p.name.clone());
            }
            Item::Subroutine(f) => {
                v.insert(f.name.clone());
            }
            _ => {}
        }
    }
    v
}
