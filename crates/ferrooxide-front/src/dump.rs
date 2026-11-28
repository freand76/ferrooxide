//! A readable, Verilog-like print of the AST (`--dump-ast`, parser snapshot tests).
//!
//! Every binary, unary and conditional expression is parenthesized, so the print shows the
//! tree's structure exactly. Loc numbers are left out (snapshots stay stable when comments
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
        BinOp::WildEq | BinOp::InsideEq => "==?",
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

/// Constraint items on one line, each ended by `;`.
pub fn citems(v: &[CItem]) -> String {
    let mut s = String::new();
    for it in v {
        s.push_str(&citem(it));
        s.push(' ');
    }
    s
}

fn citem(it: &CItem) -> String {
    let soft = |b: bool| if b { "soft " } else { "" };
    let range = |(a, b): &(Expr, Option<Expr>)| match b {
        Some(b) => format!("[{}:{}]", expr(a), expr(b)),
        None => expr(a),
    };
    match it {
        CItem::Expr { soft: s, e } => format!("{}{};", soft(*s), expr(e)),
        CItem::Inside { soft: s, e, set } => format!("{}{} inside {{{}}};", soft(*s), expr(e), set.iter().map(range).collect::<Vec<_>>().join(", ")),
        CItem::Dist { soft: s, e, items } => {
            let v: Vec<String> = items
                .iter()
                .map(|d| format!("{} {} {}", range(&(d.lo.clone(), d.hi.clone())), if d.per_value { ":=" } else { ":/" }, expr(&d.weight)))
                .collect();
            format!("{}{} dist {{{}}};", soft(*s), expr(e), v.join(", "))
        }
        CItem::Implies(g, v) => format!("{} -> {{ {}}}", expr(g), citems(v)),
        CItem::If(g, a, b) => format!("if ({}) {{ {}}} else {{ {}}}", expr(g), citems(a), citems(b)),
        CItem::Foreach { arr, vars, items } => {
            let vs: Vec<&str> = vars.iter().map(|v| v.as_deref().unwrap_or("")).collect();
            format!("foreach ({}[{}]) {{ {}}}", expr(arr), vs.join(", "), citems(items))
        }
        CItem::Unique(v) => format!("unique {{{}}};", list(v)),
        CItem::SolveBefore => "solve ... before ...;".into(),
        CItem::DisableSoft(e) => format!("disable soft {};", expr(e)),
    }
}

pub fn expr(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Num(n) => number(n),
        ExprKind::Real(r) => format!("{r:?}"),
        ExprKind::Str(s) => format!("{s:?}"),
        ExprKind::Ident(n) => n.clone(),
        ExprKind::Hier(p) => p.join("."),
        ExprKind::Index(a, i) => format!("{}[{}]", expr(a), expr(i)),
        ExprKind::Range(a, m, l) => format!("{}[{}:{}]", expr(a), expr(m), expr(l)),
        ExprKind::IndexedRange(a, b, w, up) => {
            format!("{}[{} {}: {}]", expr(a), expr(b), if *up { "+" } else { "-" }, expr(w))
        }
        ExprKind::Unary(op, a) => format!("({}{})", unop(*op), expr(a)),
        ExprKind::Binary(op, a, b) => format!("({} {} {})", expr(a), binop(*op), expr(b)),
        ExprKind::Cond(c, a, b) => format!("({} ? {} : {})", expr(c), expr(a), expr(b)),
        ExprKind::Concat(es) => format!("{{{}}}", list(es)),
        ExprKind::Repl(n, es) => format!("{{{}{{{}}}}}", expr(n), list(es)),
        ExprKind::Call(n, args) => format!("{n}({})", list(args)),
        ExprKind::SysCall(n, args) => format!("{n}({})", list(args)),
        ExprKind::Type(t) => format!("type({})", sv_type(t)),
        ExprKind::Member(a, m) => format!("{}.{m}", expr(a)),
        ExprKind::Tagged(m, None) => format!("tagged {m}"),
        ExprKind::Tagged(m, Some(v)) => format!("tagged {m} ({})", expr(v)),
        ExprKind::TagIs(a, m) => format!("$tag_is({}, {m})", expr(a)),
        ExprKind::Unchecked(a) => format!("$unchecked({})", expr(a)),
        ExprKind::NewArray(n, init) => match init {
            Some(i) => format!("new[{}]({})", expr(n), expr(i)),
            None => format!("new[{}]", expr(n)),
        },
        ExprKind::Last => "$".into(),
        ExprKind::AssignExpr { lhs, rhs } => format!("({} = {})", expr(lhs), expr(rhs)),
        ExprKind::IncDec { prefix, inc, e } => {
            let op = if *inc { "++" } else { "--" };
            if *prefix { format!("({op}{})", expr(e)) } else { format!("({}{op})", expr(e)) }
        }
        ExprKind::Stream { left, slice, items } => {
            let s = slice.as_ref().map(|s| expr(s)).unwrap_or_default();
            format!("{{{}{s}{{{}}}}}", if *left { "<<" } else { ">>" }, list(items))
        }
        ExprKind::ClassMember { class, name } => format!("{class}::{name}"),
        ExprKind::Null => "null".into(),
        ExprKind::EmptyArg => String::new(),
        ExprKind::NamedArg(n, e) => format!(".{n}({})", expr(e)),
        ExprKind::New(args) => format!("new({})", list(args)),
        ExprKind::NewTyped(c, args) => format!("{c}::new({})", list(args)),
        ExprKind::NewCopy(x) => format!("new {}", expr(x)),
        ExprKind::MethodCall(o, m, args) => format!("{}.{m}({})", expr(o), list(args)),
        ExprKind::Randomize { obj, vars, with, std, .. } => {
            let f = match (obj, std) {
                (Some(o), _) => format!("{}.randomize", expr(o)),
                (None, true) => "std::randomize".to_string(),
                (None, false) => "this.randomize".to_string(),
            };
            let w = with.as_ref().map(|v| format!(" with {{ {}}}", citems(v))).unwrap_or_default();
            format!("{f}({}){w}", list(vars))
        }
        ExprKind::ArrayWith { arr, method, depth, with } => format!("{}.{method}($it{depth}) with ({})", expr(arr), expr(with)),
        ExprKind::StaticCall(c, m, args) => format!("{c}::{m}({})", list(args)),
        ExprKind::Pattern(t, items) => {
            let items: Vec<String> = items
                .iter()
                .map(|it| match it {
                    PatItem::Pos(v) => expr(v),
                    PatItem::Key(k, v) => format!("{k}: {}", expr(v)),
                    PatItem::Index(k, v) => format!("{}: {}", expr(k), expr(v)),
                    PatItem::Default(v) => format!("default: {}", expr(v)),
                    PatItem::Type(t, v) => format!("type({}): {}", sv_type(t), expr(v)),
                    PatItem::Repl(n, v) => format!("{}{{{}}}", expr(n), list(v)),
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
        s.push_str(if f.tagged { " union tagged {" } else if f.union { " union {" } else { " struct {" });
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

pub(crate) fn event_ctl(ec: &EventCtl) -> String {
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

fn cycles(c: &Cycles) -> String {
    match (&c.hi, c.unbounded) {
        (_, true) => format!("[{}:$]", expr(&c.lo)),
        (Some(h), false) => format!("[{}:{}]", expr(&c.lo), expr(h)),
        (None, false) => expr(&c.lo),
    }
}

fn sequence(s: &Sequence) -> String {
    let rep = |c: &Cycles| match (&c.hi, c.unbounded) {
        (None, false) => format!("[{}]", expr(&c.lo)),
        _ => cycles(c),
    };
    match s {
        Sequence::Bool(e) => expr(e),
        Sequence::Delay(a, c, b) => {
            let d = match (&c.hi, c.unbounded) {
                (None, false) => format!("##{}", expr(&c.lo)),
                _ => format!("##{}", cycles(c)),
            };
            match a {
                Some(a) => format!("({} {d} {})", sequence(a), sequence(b)),
                None => format!("({d} {})", sequence(b)),
            }
        }
        Sequence::Repeat(a, c) => format!("{}[*{}]", sequence(a), rep(c).trim_start_matches('[').trim_end_matches(']')),
        Sequence::Goto(e, c) => format!("{}[->{}]", expr(e), rep(c).trim_start_matches('[').trim_end_matches(']')),
        Sequence::NonCons(e, c) => format!("{}[={}]", expr(e), rep(c).trim_start_matches('[').trim_end_matches(']')),
        Sequence::Or(a, b) => format!("({} or {})", sequence(a), sequence(b)),
        Sequence::And(a, b) => format!("({} and {})", sequence(a), sequence(b)),
        Sequence::Intersect(a, b) => format!("({} intersect {})", sequence(a), sequence(b)),
        Sequence::Throughout(e, b) => format!("({} throughout {})", expr(e), sequence(b)),
        Sequence::Within(a, b) => format!("({} within {})", sequence(a), sequence(b)),
    }
}

fn property(p: &Property) -> String {
    match p {
        Property::Seq(s) => sequence(s),
        Property::Not(a) => format!("not {}", property(a)),
        Property::And(a, b) => format!("({} and {})", property(a), property(b)),
        Property::Or(a, b) => format!("({} or {})", property(a), property(b)),
        Property::Implies { ante, next, cons } => format!("({} {} {})", sequence(ante), if *next { "|=>" } else { "|->" }, property(cons)),
        Property::If(c, a, b) => match b {
            Some(b) => format!("(if ({}) {} else {})", expr(c), property(a), property(b)),
            None => format!("(if ({}) {})", expr(c), property(a)),
        },
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
        if d.kind() == NetKind::Wire {
            s.push_str(&format!("{:?}", d.net).to_lowercase());
        } else if let Some(c) = d.ty.class.as_ref() {
            let _ = write!(s, "class {c}");
        } else {
            s.push_str(&format!("{:?}", d.kind()).to_lowercase());
        }
        if !d.explicit_kind {
            s.push_str("(implicit)");
        }
        if d.real_net {
            s.push_str(" real");
        }
        if let Some(t) = &d.type_param() {
            let _ = write!(s, " type {t}");
        }
        if let Some(t) = Some(&d.ty).filter(|t| t.has_struct()) {
            let _ = write!(s, " type({})", sv_type(t));
        }
        s.push_str(&strength(&d.strength));
        s.push_str(&delay(&d.delay));
        if d.signed() {
            s.push_str(" signed");
        }
        if let Some(r) = &d.range() {
            let _ = write!(s, " {}", range(r));
        }
        s.push_str(&ranges(&d.packed()));
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
            Item::ElabTask { name, args, .. } => {
                let a: Vec<String> = args.iter().map(|a| a.as_ref().map(expr).unwrap_or_default()).collect();
                self.line(format!("{name}({});", a.join(", ")))
            }
            Item::Assert(a) => {
                let kind = match a.kind {
                    AssertKind::Assert => "assert property",
                    AssertKind::Assume => "assume property",
                    AssertKind::Cover => "cover property",
                    AssertKind::CoverSeq => "cover sequence",
                };
                let label = a.label.as_ref().map(|l| format!("{l}: ")).unwrap_or_default();
                let dis = a.disable.as_ref().map(|d| format!("disable iff ({}) ", expr(d))).unwrap_or_default();
                self.line(format!("{label}{kind} ({} {dis}{})", event_ctl(&a.clock), property(&a.prop)));
                self.ind += 1;
                self.stmt(&a.pass);
                self.line("else");
                self.stmt(&a.fail);
                self.ind -= 1;
            }
            Item::ClassSpec { name, class, args, .. } => {
                self.line(format!("// specialization {name} = {class} #({})", args.iter().map(conn).collect::<Vec<_>>().join(", ")))
            }
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
                for p in &c.params {
                    self.line("// parameter port");
                    self.item(p);
                }
                for s in &c.specs {
                    self.item(s);
                }
                for p in &c.consts {
                    self.line(Self::param_text(p));
                }
                for d in &c.props {
                    self.line(Self::decl_text(d));
                }
                for (r, cyclic) in &c.rand {
                    self.line(format!("// {} {r}", if *cyclic { "randc" } else { "rand" }));
                }
                for k in &c.constraints {
                    self.line(format!("{}constraint {} {{ {}}}", if k.static_ { "static " } else { "" }, k.name, citems(&k.items)));
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
        if p.integer() {
            s.push_str(" integer");
        }
        if p.real() {
            s.push_str(" real");
        }
        if p.signed {
            s.push_str(" signed");
        }
        if let Some(r) = &p.range() {
            let _ = write!(s, " {}", range(r));
        }
        let dims = ranges(&p.unpacked);
        match &p.value {
            Some(v) => _ = write!(s, " {}{dims} = {};", p.name, expr(v)),
            None => _ = write!(s, " {}{dims};", p.name),
        }
        s
    }

    fn subroutine(&mut self, f: &SubroutineDecl) {
        let auto = if f.automatic { " automatic" } else { "" };
        if f.is_task {
            self.line(format!("task{auto} {};", f.name));
        } else {
            let mut ret = format!("{:?}", f.ret_kind()).to_lowercase();
            if f.signed() {
                ret.push_str(" signed");
            }
            if let Some(r) = &f.range() {
                let _ = write!(ret, " {}", range(r));
            }
            let heap = match f.ret_ty.heap.as_ref() {
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
            Stmt::Foreach { array, vars, body, .. } => {
                let v: Vec<String> = vars.iter().map(|v| v.clone().unwrap_or_default()).collect();
                self.stmt_prefixed(&format!("foreach ({}[{}])", expr(array), v.join(", ")), body)
            }
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
            Stmt::Trigger { target, nonblocking, .. } => self.line(format!("{} {};", if *nonblocking { "->>" } else { "->" }, expr(target))),
            Stmt::Return { value, .. } => match value {
                Some(v) => self.line(format!("return {};", expr(v))),
                None => self.line("return;"),
            },
            Stmt::Break { .. } => self.line("break;"),
            Stmt::Continue { .. } => self.line("continue;"),
            Stmt::Discard { e: e @ Expr { kind: ExprKind::ArrayWith { .. }, .. }, .. } => self.line(format!("{};", expr(e))),
            Stmt::Discard { e, .. } => self.line(format!("void'({});", expr(e))),
            Stmt::WaitFork { .. } => self.line("wait fork;"),
            Stmt::Deferred { final_, cover, cond, pass, fail, .. } => {
                let kw = if *cover { "cover" } else { "assert" };
                self.line(format!("{kw} {} ({})", if *final_ { "final" } else { "#0" }, expr(cond)));
                self.ind += 1;
                self.stmt(pass);
                self.line("else");
                self.stmt(fail);
                self.ind -= 1;
            }
        }
    }
}
