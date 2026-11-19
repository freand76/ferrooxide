//! A readable, Verilog-like print of the AST (`--dump-ast`, parser snapshot tests).
//!
//! Every binary, unary and conditional expression is parenthesized, so the print shows the
//! tree's structure exactly. Line numbers are left out (snapshots stay stable when comments
//! move); SystemVerilog types appear as resolved by the parser.

use crate::ast::*;
use crate::lower::SvType;
use std::fmt::Write;

/// The modules in source order.
pub fn modules(mods: &[Module]) -> String {
    let mut p = Printer { out: String::new(), ind: 0 };
    for m in mods {
        p.module(m);
    }
    p.out
}

struct Printer {
    out: String,
    ind: usize,
}

pub fn number(n: &Number) -> String {
    if n.fill {
        return format!("'{}", n.digits);
    }
    if !n.based {
        return n.digits.clone();
    }
    let b = match n.base {
        2 => 'b',
        8 => 'o',
        16 => 'h',
        _ => 'd',
    };
    let size = n.size.map(|s| s.to_string()).unwrap_or_default();
    format!("{size}'{}{b}{}", if n.signed { "s" } else { "" }, n.digits)
}

fn unop(op: UnOp) -> &'static str {
    match op {
        UnOp::Plus => "+",
        UnOp::Neg => "-",
        UnOp::Not => "~",
        UnOp::LNot => "!",
        UnOp::RedAnd => "&",
        UnOp::RedOr => "|",
        UnOp::RedXor => "^",
        UnOp::RedNand => "~&",
        UnOp::RedNor => "~|",
        UnOp::RedXnor => "~^",
    }
}

fn binop(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Pow => "**",
        BinOp::And => "&",
        BinOp::Or => "|",
        BinOp::Xor => "^",
        BinOp::Xnor => "~^",
        BinOp::LAnd => "&&",
        BinOp::LOr => "||",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::CaseEq => "===",
        BinOp::CaseNe => "!==",
        BinOp::WildEq => "==?",
        BinOp::WildNe => "!=?",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::AShl => "<<<",
        BinOp::AShr => ">>>",
    }
}

fn list(es: &[Expr]) -> String {
    es.iter().map(expr).collect::<Vec<_>>().join(", ")
}

pub fn expr(e: &Expr) -> String {
    match e {
        Expr::Num(n) => number(n),
        Expr::Real(r) => format!("{r:?}"),
        Expr::Str(s) => format!("{s:?}"),
        Expr::Ident(n) => n.clone(),
        Expr::Hier(p) => p.join("."),
        Expr::Index(a, i) => format!("{}[{}]", expr(a), expr(i)),
        Expr::Range(a, m, l) => format!("{}[{}:{}]", expr(a), expr(m), expr(l)),
        Expr::IndexedRange(a, b, w, up) => {
            format!("{}[{} {}: {}]", expr(a), expr(b), if *up { "+" } else { "-" }, expr(w))
        }
        Expr::Unary(op, a) => format!("({}{})", unop(*op), expr(a)),
        Expr::Binary(op, a, b) => format!("({} {} {})", expr(a), binop(*op), expr(b)),
        Expr::Cond(c, a, b) => format!("({} ? {} : {})", expr(c), expr(a), expr(b)),
        Expr::Concat(es) => format!("{{{}}}", list(es)),
        Expr::Repl(n, es) => format!("{{{}{{{}}}}}", expr(n), list(es)),
        Expr::Call(n, args) => format!("{n}({})", list(args)),
        Expr::SysCall(n, args) => format!("{n}({})", list(args)),
        Expr::Type(t) => format!("type({})", sv_type(t)),
        Expr::Member(a, m) => format!("{}.{m}", expr(a)),
        Expr::NewArray(n, init) => match init {
            Some(i) => format!("new[{}]({})", expr(n), expr(i)),
            None => format!("new[{}]", expr(n)),
        },
        Expr::Last => "$".into(),
        Expr::IncDec { prefix, inc, e } => {
            let op = if *inc { "++" } else { "--" };
            if *prefix { format!("({op}{})", expr(e)) } else { format!("({}{op})", expr(e)) }
        }
        Expr::Null => "null".into(),
        Expr::EmptyArg => String::new(),
        Expr::NamedArg(n, e) => format!(".{n}({})", expr(e)),
        Expr::New(args) => format!("new({})", list(args)),
        Expr::NewTyped(c, args) => format!("{c}::new({})", list(args)),
        Expr::NewCopy(x) => format!("new {}", expr(x)),
        Expr::MethodCall(o, m, args) => format!("{}.{m}({})", expr(o), list(args)),
        Expr::StaticCall(c, m, args) => format!("{c}::{m}({})", list(args)),
        Expr::Pattern(t, items) => {
            let items: Vec<String> = items
                .iter()
                .map(|it| match it {
                    PatItem::Pos(v) => expr(v),
                    PatItem::Key(k, v) => format!("{k}: {}", expr(v)),
                    PatItem::Index(k, v) => format!("{}: {}", expr(k), expr(v)),
                    PatItem::Default(v) => format!("default: {}", expr(v)),
                })
                .collect();
            let t = t.as_ref().map(|t| format!("type({})", sv_type(t))).unwrap_or_default();
            format!("{t}'{{{}}}", items.join(", "))
        }
    }
}

fn range(r: &Range) -> String {
    format!("[{}:{}]", expr(&r.0), expr(&r.1))
}

fn ranges(rs: &[Range]) -> String {
    rs.iter().map(range).collect()
}

pub fn sv_type(t: &SvType) -> String {
    let mut s = format!("{:?}", t.kind).to_lowercase();
    if t.signed {
        s.push_str(" signed");
    }
    if let Some(p) = &t.param {
        let _ = write!(s, " param {p}");
    }
    if let Some(f) = &t.fields {
        if !f.packed {
            s.push_str(" unpacked");
        }
        s.push_str(if f.union { " union {" } else { " struct {" });
        for (n, ft) in &f.fields {
            let _ = write!(s, " {} {n};", sv_type(ft));
        }
        s.push_str(" }");
    }
    if let Some(names) = &t.enum_names {
        let _ = write!(s, " enum {{{}}}", names.join(", "));
    }
    if !t.dims.is_empty() && !t.int_kind {
        let _ = write!(s, " {}", ranges(&t.dims));
    }
    if !t.unpacked.is_empty() {
        let _ = write!(s, " unpacked {}", ranges(&t.unpacked));
    }
    s
}

fn strength(s: &Option<(u8, u8)>) -> String {
    match s {
        Some((a, b)) => format!(" (strength {a}, {b})"),
        None => String::new(),
    }
}

fn delay(d: &Option<Expr>) -> String {
    match d {
        Some(d) => format!(" #({})", expr(d)),
        None => String::new(),
    }
}

fn conn(c: &Conn) -> String {
    match c {
        Conn::Positional(e) => e.as_ref().map(expr).unwrap_or_default(),
        Conn::Named(n, e) => format!(".{n}({})", e.as_ref().map(expr).unwrap_or_default()),
        Conn::Wildcard => ".*".into(),
    }
}

fn event_ctl(ec: &EventCtl) -> String {
    match ec {
        EventCtl::Star => "@*".into(),
        EventCtl::List(l) => {
            let parts: Vec<String> = l
                .iter()
                .map(|(edge, e)| {
                    let k = match edge {
                        Edge::Any => "",
                        Edge::Pos => "posedge ",
                        Edge::Neg => "negedge ",
                    };
                    format!("{k}{}", expr(e))
                })
                .collect();
            format!("@({})", parts.join(" or "))
        }
    }
}

fn timing(t: &Timing) -> String {
    match t {
        Timing::Delay(d) => format!("#({})", expr(d)),
        Timing::Event(ec) => event_ctl(ec),
        Timing::Repeat(n, ec) => format!("repeat ({}) {}", expr(n), event_ctl(ec)),
    }
}

/// A blocking assignment without timing as `lhs = rhs` (for a `for` header).
fn simple_assign(s: &Stmt) -> Option<String> {
    match s {
        Stmt::Assign { lhs, rhs, nonblocking: false, timing: None, .. } => Some(format!("{} = {}", expr(lhs), expr(rhs))),
        _ => None,
    }
}

fn name(n: &Option<String>) -> String {
    match n {
        Some(n) => format!(" : {n}"),
        None => String::new(),
    }
}

impl Printer {
    fn line(&mut self, s: impl AsRef<str>) {
        for _ in 0..self.ind {
            self.out.push_str("  ");
        }
        self.out.push_str(s.as_ref());
        self.out.push('\n');
    }

    fn module(&mut self, m: &Module) {
        let kind = if m.udp.is_some() {
            "primitive"
        } else if m.package {
            "package"
        } else if m.interface {
            "interface"
        } else if m.program {
            "program"
        } else {
            "module"
        };
        self.line(format!("{kind} {} ({});", m.name, m.ports.join(", ")));
        self.ind += 1;
        if let Some((u, p)) = m.timescale {
            self.line(format!("// timescale 1e{u} / 1e{p}"));
        }
        if let Some(n) = &m.default_nettype {
            self.line(format!("// default_nettype {n}"));
        }
        if let Some(d) = m.unconnected_drive {
            self.line(format!("// unconnected_drive pull{}", d as u8));
        }
        if !m.imports.is_empty() {
            self.line(format!("// imports {}", m.imports.join(", ")));
        }
        for (p, i, r, mp) in &m.iface_ports {
            let r = r.as_ref().map(range).unwrap_or_default();
            let mp = mp.as_ref().map(|m| format!(".{m}")).unwrap_or_default();
            self.line(format!("// interface port {i}{mp} {p}{r}"));
        }
        if !m.modports.is_empty() {
            self.line(format!("// modports {}", m.modports.join(", ")));
        }
        if !m.keyword_names.is_empty() {
            // each name once, where it first appears (what the lint reports)
            let mut names: Vec<&str> = Vec::new();
            for (n, _) in &m.keyword_names {
                if !names.contains(&n.as_str()) {
                    names.push(n);
                }
            }
            self.line(format!("// keyword names {}", names.join(", ")));
        }
        if let Some(u) = &m.udp {
            if let Some(i) = &u.init {
                self.line(format!("initial {};", expr(i)));
            }
            self.line(if u.sequential { "table // sequential" } else { "table" });
            for (ins, cur, out) in &u.rows {
                let cur = cur.as_ref().map(|c| format!(" : {c}")).unwrap_or_default();
                self.line(format!("  {}{cur} : {out};", ins.join(" ")));
            }
            self.line("endtable");
        }
        self.items(&m.items);
        for n in &m.nested {
            self.module(n);
        }
        self.ind -= 1;
        self.line(format!("end{kind}"));
    }

    fn items(&mut self, items: &[Item]) {
        for it in items {
            self.item(it);
        }
    }

    fn decl_text(d: &Decl) -> String {
        let mut s = String::new();
        if d.static_ {
            s.push_str("static ");
        }
        if d.automatic {
            s.push_str("automatic ");
        }
        if let Some(dir) = d.dir {
            s.push_str(match dir {
                Dir::Input => "input ",
                Dir::Output => "output ",
                Dir::Inout => "inout ",
            });
        }
        if d.kind == NetKind::Wire {
            s.push_str(&format!("{:?}", d.net).to_lowercase());
        } else if let Some(c) = d.ty.as_ref().and_then(|t| t.class.as_ref()) {
            let _ = write!(s, "class {c}");
        } else {
            s.push_str(&format!("{:?}", d.kind).to_lowercase());
        }
        if !d.explicit_kind {
            s.push_str("(implicit)");
        }
        if d.real_net {
            s.push_str(" real");
        }
        if let Some(t) = &d.type_param {
            let _ = write!(s, " type {t}");
        }
        if let Some(t) = d.ty.as_ref().filter(|t| t.has_struct()) {
            let _ = write!(s, " type({})", sv_type(t));
        }
        s.push_str(&strength(&d.strength));
        s.push_str(&delay(&d.delay));
        if d.signed {
            s.push_str(" signed");
        }
        if let Some(r) = &d.range {
            let _ = write!(s, " {}", range(r));
        }
        s.push_str(&ranges(&d.packed));
        let names: Vec<String> = d
            .names
            .iter()
            .map(|n| {
                let init = n.init.as_ref().map(|i| format!(" = {}", expr(i))).unwrap_or_default();
                let heap = match &n.heap {
                    None => String::new(),
                    Some(HeapKind::Dynamic) => "[]".into(),
                    Some(HeapKind::Queue(None)) => "[$]".into(),
                    Some(HeapKind::Queue(Some(m))) => format!("[$:{}]", expr(m)),
                Some(HeapKind::Assoc(None)) => "[*]".into(),
                Some(HeapKind::Assoc(Some(k))) => format!("[{}]", sv_type(k)),
                };
                format!("{}{}{heap}{init}", n.name, ranges(&n.dims))
            })
            .collect();
        let _ = write!(s, " {};", names.join(", "));
        s
    }

    fn item(&mut self, it: &Item) {
        match it {
            Item::Decl(d) => self.line(Self::decl_text(d)),
            Item::Param(p) => self.line(Self::param_text(p)),
            Item::TypeParam { local, name, default, .. } => {
                let kw = if *local { "localparam" } else { "parameter" };
                let d = default.as_ref().map(|t| format!(" = {}", sv_type(t))).unwrap_or_default();
                self.line(format!("{kw} type {name}{d};"));
            }
            Item::SpecPath { srcs, dsts, delay, .. } => {
                self.line(format!("specify ({} => {}) = {};", srcs.join(", "), dsts.join(", "), expr(delay)))
            }
            Item::Defparam { target, value, .. } => {
                self.line(format!("defparam {} = {};", target.join("."), expr(value)))
            }
            Item::ContAssign { delay: d, lhs, rhs, strength: st, .. } => {
                self.line(format!("assign{}{} {} = {};", strength(st), delay(d), expr(lhs), expr(rhs)))
            }
            Item::Process { kind, body, .. } => {
                let k = match kind {
                    ProcKind::Initial => "initial",
                    ProcKind::Always => "always",
                    ProcKind::AlwaysComb => "always_comb",
                    ProcKind::AlwaysFf => "always_ff",
                    ProcKind::AlwaysLatch => "always_latch",
                    ProcKind::Final => "final",
                };
                self.stmt_prefixed(k, body);
            }
            Item::Instances { module, params, insts, .. } => {
                let ps = if params.is_empty() {
                    String::new()
                } else {
                    format!(" #({})", params.iter().map(conn).collect::<Vec<_>>().join(", "))
                };
                for i in insts {
                    let cs: Vec<String> = i.conns.iter().map(conn).collect();
                    self.line(format!("{module}{ps} {}{}({});", i.name, ranges(&i.dims), cs.join(", ")));
                }
            }
            Item::Subroutine(f) => self.subroutine(f),
            Item::Class(c) => {
                let mut head = format!("{}class {}", if c.abstract_ { "virtual " } else { "" }, c.name);
                if let Some(b) = &c.base {
                    let _ = write!(head, " extends {b}");
                    if let Some(a) = &c.base_args {
                        let _ = write!(head, "({})", list(a));
                    }
                }
                self.line(format!("{head};"));
                self.ind += 1;
                for p in &c.consts {
                    self.line(Self::param_text(p));
                }
                for d in &c.props {
                    self.line(Self::decl_text(d));
                }
                for f in &c.methods {
                    if c.virtuals.contains(&f.name) {
                        self.line("// virtual");
                    }
                    if c.statics.contains(&f.name) {
                        self.line("// static");
                    }
                    self.subroutine(f);
                }
                self.ind -= 1;
                self.line("endclass");
            }
            Item::GenFor { init_var, init, cond, step_var, step, name: n, items, .. } => {
                self.line(format!(
                    "for ({init_var} = {}; {}; {step_var} = {}) begin{}",
                    expr(init),
                    expr(cond),
                    expr(step),
                    name(n)
                ));
                self.gen_items(items);
            }
            Item::GenIf { cond, then_items, then_name, else_items, else_name, .. } => {
                self.line(format!("if ({}) begin{}", expr(cond), name(then_name)));
                self.gen_items(then_items);
                if !else_items.is_empty() || else_name.is_some() {
                    self.line(format!("else begin{}", name(else_name)));
                    self.gen_items(else_items);
                }
            }
            Item::GenCase { expr: e, arms, .. } => {
                self.line(format!("case ({})", expr(e)));
                self.ind += 1;
                for (labels, n, items) in arms {
                    let l = if labels.is_empty() { "default".to_string() } else { list(labels) };
                    self.line(format!("{l}: begin{}", name(n)));
                    self.gen_items(items);
                }
                self.ind -= 1;
                self.line("endcase");
            }
            Item::GenBlock { name: n, items, .. } => {
                self.line(format!("begin{}", name(n)));
                self.gen_items(items);
            }
            Item::Gate { kind, inst_name, dims, terms, strength: st, delay: d, .. } => {
                let n = inst_name.clone().unwrap_or_default();
                let r = dims.as_ref().map(range).unwrap_or_default();
                self.line(format!("{kind}{}{} {n}{r}({});", strength(st), delay(d), list(terms)));
            }
        }
    }

    fn gen_items(&mut self, items: &[Item]) {
        self.ind += 1;
        self.items(items);
        self.ind -= 1;
        self.line("end");
    }

    fn param_text(p: &ParamDecl) -> String {
        let kw = if p.local { "localparam" } else { "parameter" };
        let mut s = kw.to_string();
        if p.integer {
            s.push_str(" integer");
        }
        if p.real {
            s.push_str(" real");
        }
        if p.signed {
            s.push_str(" signed");
        }
        if let Some(r) = &p.range {
            let _ = write!(s, " {}", range(r));
        }
        match &p.value {
            Some(v) => _ = write!(s, " {} = {};", p.name, expr(v)),
            None => _ = write!(s, " {};", p.name),
        }
        s
    }

    fn subroutine(&mut self, f: &SubroutineDecl) {
        let auto = if f.automatic { " automatic" } else { "" };
        if f.is_task {
            self.line(format!("task{auto} {};", f.name));
        } else {
            let mut ret = format!("{:?}", f.ret_kind).to_lowercase();
            if f.signed {
                ret.push_str(" signed");
            }
            if let Some(r) = &f.range {
                let _ = write!(ret, " {}", range(r));
            }
            let heap = match f.ret_ty.as_ref().and_then(|t| t.heap.as_ref()) {
                None => String::new(),
                Some(HeapKind::Dynamic) => "[]".into(),
                Some(HeapKind::Queue(None)) => "[$]".into(),
                Some(HeapKind::Queue(Some(m))) => format!("[$:{}]", expr(m)),
                Some(HeapKind::Assoc(None)) => "[*]".into(),
                Some(HeapKind::Assoc(Some(k))) => format!("[{}]", sv_type(k)),
            };
            self.line(format!("function{auto} {ret} {}{heap};", f.name));
        }
        self.ind += 1;
        for p in &f.params {
            self.line(Self::param_text(p));
        }
        for d in &f.decls {
            self.line(Self::decl_text(d));
        }
        self.stmt(&f.body);
        self.ind -= 1;
        self.line(if f.is_task { "endtask" } else { "endfunction" });
    }

    /// `prefix` followed by the statement (on the same line for a block).
    fn stmt_prefixed(&mut self, prefix: &str, s: &Stmt) {
        match s {
            Stmt::Block { .. } => {
                let start = self.out.len();
                self.stmt(s);
                // put the prefix in front of the block's first line
                let first = &self.out[start..];
                let indent = first.len() - first.trim_start().len();
                self.out.insert_str(start + indent, &format!("{prefix} "));
            }
            _ => {
                self.line(prefix);
                self.ind += 1;
                self.stmt(s);
                self.ind -= 1;
            }
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Null => self.line(";"),
            Stmt::Block { name: n, decls, imports, params, stmts, fork } => {
                let (open, close) = match fork {
                    None => ("begin", "end"),
                    Some(Join::All) => ("fork", "join"),
                    Some(Join::Any) => ("fork", "join_any"),
                    Some(Join::None) => ("fork", "join_none"),
                };
                self.line(format!("{open}{}", name(n)));
                self.ind += 1;
                for i in imports {
                    self.line(if i.contains("::") { format!("import {i};") } else { format!("import {i}::*;") });
                }
                for p in params {
                    self.line(Self::param_text(p));
                }
                for d in decls {
                    self.line(Self::decl_text(d));
                }
                for s in stmts {
                    self.stmt(s);
                }
                self.ind -= 1;
                self.line(close);
            }
            Stmt::Assign { lhs, rhs, nonblocking, timing: t, .. } => {
                let op = if *nonblocking { "<=" } else { "=" };
                let t = t.as_ref().map(|t| format!("{} ", timing(t))).unwrap_or_default();
                self.line(format!("{} {op} {t}{};", expr(lhs), expr(rhs)));
            }
            Stmt::If { cond, then_s, else_s, .. } => {
                self.stmt_prefixed(&format!("if ({})", expr(cond)), then_s);
                if let Some(e) = else_s {
                    self.stmt_prefixed("else", e);
                }
            }
            Stmt::Case { kind, expr: e, arms, default, .. } => {
                let k = match kind {
                    CaseKind::Case => "case",
                    CaseKind::CaseZ => "casez",
                    CaseKind::CaseX => "casex",
                };
                self.line(format!("{k} ({})", expr(e)));
                self.ind += 1;
                for (labels, body) in arms {
                    self.stmt_prefixed(&format!("{}:", list(labels)), body);
                }
                if let Some(d) = default {
                    self.stmt_prefixed("default:", d);
                }
                self.ind -= 1;
                self.line("endcase");
            }
            Stmt::For { init, cond, step, body, .. } if simple_assign(init).is_some() && simple_assign(step).is_some() => {
                let head = format!(
                    "for ({}; {}; {})",
                    simple_assign(init).unwrap(),
                    expr(cond),
                    simple_assign(step).unwrap()
                );
                self.stmt_prefixed(&head, body);
            }
            Stmt::For { init, cond, step, body, .. } => {
                self.line("for");
                self.ind += 1;
                self.stmt_prefixed("init:", init);
                self.line(format!("cond: {}", expr(cond)));
                self.stmt_prefixed("step:", step);
                self.ind -= 1;
                self.stmt_prefixed("do", body);
            }
            Stmt::While { cond, body, .. } => self.stmt_prefixed(&format!("while ({})", expr(cond)), body),
            Stmt::DoWhile { cond, body, .. } => {
                self.stmt_prefixed("do", body);
                self.line(format!("while ({});", expr(cond)));
            }
            Stmt::Repeat { count, body, .. } => self.stmt_prefixed(&format!("repeat ({})", expr(count)), body),
            Stmt::Forever { body, .. } => self.stmt_prefixed("forever", body),
            Stmt::Timed { timing: t, body, .. } => self.stmt_prefixed(&timing(t), body),
            Stmt::Wait { cond, body, .. } => self.stmt_prefixed(&format!("wait ({})", expr(cond)), body),
            Stmt::SysTask { name: n, args, .. } => {
                let a: Vec<String> = args.iter().map(|a| a.as_ref().map(expr).unwrap_or_default()).collect();
                self.line(format!("{n}({});", a.join(", ")));
            }
            Stmt::TaskCall { name: n, args, .. } => self.line(format!("{n}({});", list(args))),
            Stmt::MethodCall { obj, name: n, args, .. } => self.line(format!("{}.{n}({});", expr(obj), list(args))),
            Stmt::StaticCall { class, name: n, args, .. } => self.line(format!("{class}::{n}({});", list(args))),
            Stmt::Disable { name: n, .. } => self.line(format!("disable {n};")),
            Stmt::Force { lhs, rhs: Some(r), .. } => self.line(format!("force {} = {};", expr(lhs), expr(r))),
            Stmt::Force { lhs, rhs: None, .. } => self.line(format!("release {};", expr(lhs))),
            Stmt::Trigger { name: n, .. } => self.line(format!("-> {n};")),
            Stmt::Return { value, .. } => match value {
                Some(v) => self.line(format!("return {};", expr(v))),
                None => self.line("return;"),
            },
            Stmt::Break { .. } => self.line("break;"),
            Stmt::Continue { .. } => self.line("continue;"),
            Stmt::Discard { e, .. } => self.line(format!("void'({});", expr(e))),
            Stmt::WaitFork { .. } => self.line("wait fork;"),
        }
    }
}
