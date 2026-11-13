//! Recursive-descent parser for Verilog-2005 plus common SystemVerilog constructs.

use crate::ast::*;
use crate::lexer::{Tok, Token};
use crate::{Error, Result};

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    file: String,
    /// `timescale in effect (carried from file to file by the caller)
    pub timescale: Option<(i32, i32)>,
}

struct PType {
    signed: bool,
    integer: bool,
    real: bool,
    range: Option<Range>,
}

const KEYWORDS: &[&str] = &[
    "module", "endmodule", "macromodule", "input", "output", "inout", "wire", "reg", "integer", "time", "real",
    "parameter", "localparam", "assign", "always", "initial", "begin", "end", "if", "else", "case", "casez",
    "casex", "endcase", "default", "for", "while", "repeat", "forever", "posedge", "negedge", "or", "function",
    "endfunction", "task", "endtask", "generate", "endgenerate", "genvar", "signed", "unsigned", "fork", "join",
    "wait", "disable", "tri", "supply0", "supply1", "wand", "wor", "logic", "bit", "int", "byte", "shortint",
    "longint", "always_comb", "always_ff", "always_latch", "automatic", "return", "do", "final", "defparam",
    "specify", "endspecify", "join_any", "force", "release", "deassign", "realtime", "event", "join_none", "break", "continue", "unique", "priority",
];

fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

const GATES: &[&str] = &[
    "and", "or", "nand", "nor", "xor", "xnor", "not", "buf", "bufif0", "bufif1", "notif0", "notif1", "nmos", "pmos",
    "rnmos", "rpmos", "cmos", "rcmos", "pullup", "pulldown",
];

const STRENGTHS: &[&str] = &[
    "supply0", "supply1", "strong0", "strong1", "pull0", "pull1", "weak0", "weak1", "highz0", "highz1", "small",
    "medium", "large",
];

impl Parser {
    pub fn new(toks: Vec<Token>, file: String) -> Self {
        Parser { toks, pos: 0, file, timescale: None }
    }

    // ---------- token helpers ----------
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn peek_at(&self, k: usize) -> &Tok {
        let i = (self.pos + k).min(self.toks.len() - 1);
        &self.toks[i].tok
    }
    fn line(&self) -> u32 {
        self.toks[self.pos].line
    }
    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn err<T>(&self, msg: impl Into<String>) -> Result<T> {
        Err(Error { msg: msg.into(), file: self.file.clone(), line: self.line() })
    }
    fn is_p(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }
    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Tok::Ident(q) if q == k)
    }
    fn eat_p(&mut self, p: &str) -> bool {
        if self.is_p(p) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn eat_kw(&mut self, k: &str) -> bool {
        if self.is_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn expect_p(&mut self, p: &str) -> Result<()> {
        if self.eat_p(p) {
            Ok(())
        } else {
            self.err(format!("expected '{p}', found {}", self.describe()))
        }
    }
    fn expect_kw(&mut self, k: &str) -> Result<()> {
        if self.eat_kw(k) {
            Ok(())
        } else {
            self.err(format!("expected '{k}', found {}", self.describe()))
        }
    }
    fn describe(&self) -> String {
        match self.peek() {
            Tok::Ident(s) => format!("'{s}'"),
            Tok::SysIdent(s) => format!("'{s}'"),
            Tok::Num(_) | Tok::Real(_) => "number".into(),
            Tok::Str(_) => "string".into(),
            Tok::Punct(p) => format!("'{p}'"),
            Tok::Eof => "end of file".into(),
        }
    }
    fn ident(&mut self) -> Result<String> {
        match self.peek().clone() {
            Tok::Ident(s) if !is_keyword(&s) => {
                self.bump();
                Ok(s)
            }
            _ => self.err(format!("expected identifier, found {}", self.describe())),
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

    fn opt_block_label(&mut self) -> Result<Option<String>> {
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
                Tok::Ident(s) if s == "module" || s == "macromodule" => mods.push(self.module()?),
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
        self.bump();
        let name = self.ident()?;
        let mut items = Vec::new();
        let mut ports = Vec::new();
        // parameter port list
        if self.eat_p("#") {
            self.expect_p("(")?;
            let mut local = false;
            let mut pt = PType { signed: false, integer: false, real: false, range: None };
            loop {
                if self.eat_kw("parameter") {
                    local = false;
                    pt = self.param_type()?;
                } else if self.eat_kw("localparam") {
                    local = true;
                    pt = self.param_type()?;
                } else if matches!(self.peek(), Tok::Ident(s) if s == "integer" || s == "signed" || s == "int" || s == "logic" || s == "bit" || s == "real")
                    || self.is_p("[")
                {
                    pt = self.param_type()?;
                }
                let pline = self.line();
                let pname = self.ident()?;
                self.expect_p("=")?;
                let value = self.expr()?;
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
                if matches!(self.peek(), Tok::Ident(s) if s == "input" || s == "output" || s == "inout") {
                    // ANSI style
                    let mut cur: Option<Decl> = None;
                    loop {
                        let pline = self.line();
                        if matches!(self.peek(), Tok::Ident(s) if s == "input" || s == "output" || s == "inout") {
                            let d = self.decl_head(true)?;
                            cur = Some(d);
                        } else if cur.is_none() {
                            return self.err("expected port direction");
                        } else if matches!(self.peek(), Tok::Ident(s) if is_keyword(s)) || self.is_p("[") {
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
        while !self.is_kw("endmodule") {
            if matches!(self.peek(), Tok::Eof) {
                return self.err("unexpected end of file in module");
            }
            self.module_item(&mut items)?;
        }
        self.bump();
        self.opt_block_label()?;
        Ok(Module { name, timescale: self.timescale, file: self.file.clone(), line, ports, items })
    }

    fn param_type(&mut self) -> Result<PType> {
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
        Ok(PType { signed, integer, real, range })
    }

    fn range(&mut self) -> Result<Range> {
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
        let mut kind = NetKind::Wire;
        let mut explicit_kind = false;
        let mut signed = false;
        let mut net = NetType::Wire;
        if let Tok::Ident(s) = self.peek().clone() {
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
                // `wire logic` / `var logic`
                if self.is_kw("logic") || self.is_kw("reg") {
                    self.bump();
                }
                // drive / charge strength of a net declaration
                self.skip_strength()?;
            }
        }
        if self.eat_kw("signed") {
            signed = true;
        } else if self.eat_kw("unsigned") {
            signed = false;
        }
        let range = if self.is_p("[") { Some(self.range()?) } else { None };
        // optional net delay: wire #5 w;
        if self.eat_p("#") {
            self.delay_value()?;
        }
        Ok(Decl { dir, kind, net, explicit_kind, signed, range, names: Vec::new(), line })
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
            let init = if self.eat_p("=") { Some(self.expr()?) } else { None };
            v.push(Declarator { name, dims, init, line });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(";")?;
        Ok(v)
    }

    fn is_decl_start(&self) -> bool {
        matches!(self.peek(), Tok::Ident(s) if matches!(s.as_str(),
            "input"|"output"|"inout"|"wire"|"reg"|"logic"|"bit"|"integer"|"time"|"genvar"|"int"|"byte"|"shortint"|"longint"|"real"|"realtime"|"event"
            |"tri"|"wand"|"wor"|"triand"|"trior"|"trireg"|"supply0"|"supply1"|"tri0"|"tri1"|"uwire"))
    }

    fn param_items(&mut self, local: bool) -> Result<Vec<ParamDecl>> {
        let pt = self.param_type()?;
        let mut v = Vec::new();
        loop {
            let line = self.line();
            let name = self.ident()?;
            self.expect_p("=")?;
            let value = self.expr()?;
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

    fn module_item(&mut self, items: &mut Vec<Item>) -> Result<()> {
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
            "parameter" | "localparam" => {
                self.bump();
                for p in self.param_items(kw == "localparam")? {
                    items.push(Item::Param(p));
                }
            }
            "assign" => {
                self.bump();
                self.skip_strength()?;
                let delay = if self.eat_p("#") { Some(self.delay_value()?) } else { None };
                loop {
                    let line = self.line();
                    let lhs = self.lvalue()?;
                    self.expect_p("=")?;
                    let rhs = self.expr()?;
                    items.push(Item::ContAssign { delay: delay.clone(), lhs, rhs, line });
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
            "defparam" => return self.err("defparam is not supported"),
            "specify" => {
                while !self.eat_kw("endspecify") {
                    if matches!(self.peek(), Tok::Eof) {
                        return self.err("unterminated specify");
                    }
                    self.bump();
                }
            }
            _ if self.is_decl_start() => {
                let mut d = self.decl_head(true)?;
                d.names = self.declarators()?;
                items.push(Item::Decl(d));
            }
            _ if GATES.contains(&kw) => {
                self.bump();
                let kind = kw.to_string();
                self.skip_strength()?;
                if self.eat_p("#") {
                    self.delay_value()?;
                }
                loop {
                    let line = self.line();
                    let inst_name = if let Tok::Ident(_) = self.peek() { Some(self.ident()?) } else { None };
                    self.expect_p("(")?;
                    let mut terms = Vec::new();
                    loop {
                        terms.push(self.expr()?);
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                    self.expect_p(")")?;
                    items.push(Item::Gate { kind: kind.clone(), inst_name, terms, line });
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
            }
            _ if !is_keyword(kw) => {
                // module instantiation
                let module = self.ident()?;
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
                    let name = self.ident()?;
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
        if !is_task {
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
        let mut decls = Vec::new();
        if self.eat_p("(") {
            if !self.is_p(")") {
                let mut cur: Option<Decl> = None;
                loop {
                    if matches!(self.peek(), Tok::Ident(s) if s == "input" || s == "output" || s == "inout")
                        || cur.is_none()
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
        Ok(SubroutineDecl { name, is_task, automatic, signed, range, ret_kind, decls, body, line })
    }

    // ---------- statements ----------
    /// `( strength0, strength1 )` or a charge strength `(small)`: skipped (no strength model)
    fn skip_strength(&mut self) -> Result<()> {
        if self.is_p("(") && matches!(self.peek_at(1), Tok::Ident(s) if STRENGTHS.contains(&s.as_str())) {
            while !self.eat_p(")") {
                if matches!(self.peek(), Tok::Eof) {
                    return self.err("unterminated strength");
                }
                self.bump();
            }
        }
        Ok(())
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
            // (rise, fall[, turn-off]): the rise delay is used
            while self.eat_p(",") {
                mtm(self)?;
            }
            self.expect_p(")")?;
            Ok(e)
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
            let id = self.ident()?;
            return Ok(EventCtl::List(vec![(Edge::Any, Expr::Ident(id))]));
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
                    let value = if self.is_p(";") { None } else { Some(self.expr()?) };
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
        } else {
            None
        };
        let rhs = self.expr()?;
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
        let base = self.hier_tail(name)?;
        self.selects(base)
    }

    fn hier_tail(&mut self, name: String) -> Result<Expr> {
        if self.is_p(".") && matches!(self.peek_at(1), Tok::Ident(_)) {
            let mut parts = vec![name];
            while self.eat_p(".") {
                parts.push(self.ident()?);
            }
            return Ok(Expr::Hier(parts));
        }
        Ok(Expr::Ident(name))
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
        let t = self.bump();
        let e = match t {
            Tok::Num(n) => Expr::Num(n),
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
            Tok::SysIdent(name) => {
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
                Expr::SysCall(name, args)
            }
            Tok::Ident(name) if !is_keyword(&name) => {
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

pub fn dec(v: u64) -> Number {
    Number { size: None, signed: true, base: 10, based: false, digits: v.to_string(), fill: false }
}
