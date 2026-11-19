//! Expressions: names with their selects, members and calls (struct member chains, enum
//! methods, hierarchical names with `name[N]` segments, package items), casts, patterns,
//! `inside`, array queries, `$bits` of types.

use super::types::Op;
use super::Lower;
use crate::ast::*;
use super::push_hier_index;
use super::types::cast;
use crate::syntax::tree as t;
use crate::Result;
use std::rc::Rc;

/// The start of a postfix chain.
#[derive(Clone, Copy)]
enum Base<'a> {
    Name(&'a str, t::Loc),
    Other(&'a t::Expr),
}

/// A name with its postfix steps, innermost first.
fn flatten(e: &t::Expr) -> (Base<'_>, Vec<Op<'_>>) {
    let mut ops = Vec::new();
    let mut cur = e;
    loop {
        match &cur.kind {
            t::ExprKind::Member(a, m) => {
                // `$root.a`: a name from the top level (the upward search finds it)
                if matches!(a.kind, t::ExprKind::Root) {
                    ops.reverse();
                    return (Base::Name(m, cur.loc), ops);
                }
                ops.push(Op::Member(m, cur.loc));
                cur = a;
            }
            t::ExprKind::Index(a, i) => {
                ops.push(Op::Index(i));
                cur = a;
            }
            t::ExprKind::Range(a, k, x, y) => {
                ops.push(Op::Range(*k, x, y));
                cur = a;
            }
            t::ExprKind::Call { func, args, with_: None, parens: true } => {
                ops.push(Op::Call(args, cur.loc));
                cur = func;
            }
            t::ExprKind::Ident(n) => {
                ops.reverse();
                return (Base::Name(n, cur.loc), ops);
            }
            t::ExprKind::This | t::ExprKind::Super => {
                ops.reverse();
                let n = if matches!(cur.kind, t::ExprKind::This) { "this" } else { "super" };
                return (Base::Name(n, cur.loc), ops);
            }
            _ => {
                ops.reverse();
                return (Base::Other(cur), ops);
            }
        }
    }
}

fn binop(op: t::BinOp) -> Option<BinOp> {
    use t::BinOp as B;
    Some(match op {
        B::Add => BinOp::Add,
        B::Sub => BinOp::Sub,
        B::Mul => BinOp::Mul,
        B::Div => BinOp::Div,
        B::Mod => BinOp::Mod,
        B::Pow => BinOp::Pow,
        B::And => BinOp::And,
        B::Or => BinOp::Or,
        B::Xor => BinOp::Xor,
        B::Xnor => BinOp::Xnor,
        B::LAnd => BinOp::LAnd,
        B::LOr => BinOp::LOr,
        B::Eq => BinOp::Eq,
        B::Ne => BinOp::Ne,
        B::CaseEq => BinOp::CaseEq,
        B::CaseNe => BinOp::CaseNe,
        B::WildEq => BinOp::WildEq,
        B::WildNe => BinOp::WildNe,
        B::Lt => BinOp::Lt,
        B::Le => BinOp::Le,
        B::Gt => BinOp::Gt,
        B::Ge => BinOp::Ge,
        B::Shl => BinOp::Shl,
        B::Shr => BinOp::Shr,
        B::AShl => BinOp::AShl,
        B::AShr => BinOp::AShr,
        B::LImpl | B::LEquiv => return None,
    })
}

fn unop(op: t::UnOp) -> UnOp {
    use t::UnOp as U;
    match op {
        U::Plus => UnOp::Plus,
        U::Neg => UnOp::Neg,
        U::Not => UnOp::Not,
        U::LNot => UnOp::LNot,
        U::RedAnd => UnOp::RedAnd,
        U::RedOr => UnOp::RedOr,
        U::RedXor => UnOp::RedXor,
        U::RedNand => UnOp::RedNand,
        U::RedNor => UnOp::RedNor,
        U::RedXnor => UnOp::RedXnor,
    }
}

const ENUM_METHODS: &[&str] = &["first", "last", "num", "next", "prev", "name"];
const ARRAY_QUERIES: &[&str] = &["$left", "$right", "$low", "$high", "$size", "$increment", "$dimensions", "$unpacked_dimensions"];

/// The value of a literal index (no x/z), possibly negated.
pub(crate) fn lit_index(e: &Expr) -> Option<i64> {
    match e {
        Expr::Num(n) if !n.digits.contains(['x', 'z', 'X', 'Z', '?']) => i64::from_str_radix(&n.digits.replace('_', ""), n.base).ok(),
        Expr::Unary(UnOp::Neg, b) => match &**b {
            Expr::Num(n) => i64::from_str_radix(&n.digits.replace('_', ""), n.base).ok().map(|v| -v),
            _ => None,
        },
        _ => None,
    }
}

impl Lower {
    pub(crate) fn expr(&mut self, e: &t::Expr) -> Result<Expr> {
        let loc = e.loc;
        Ok(match &e.kind {
            t::ExprKind::Num(n) => Expr::Num(n.clone()),
            t::ExprKind::Real(r) => Expr::Real(*r),
            t::ExprKind::Time(v, u) => self.time_literal(*v, *u),
            t::ExprKind::Str(s) => Expr::Str(s.clone()),
            t::ExprKind::Ident(_)
            | t::ExprKind::Member(..)
            | t::ExprKind::Index(..)
            | t::ExprKind::Range(..)
            | t::ExprKind::Scoped(_)
            | t::ExprKind::This
            | t::ExprKind::Super => {
                let (b, ops) = flatten(e);
                self.chain(b, &ops, false)?
            }
            t::ExprKind::Call { with_: None, parens: true, .. } => {
                let (b, ops) = flatten(e);
                self.chain(b, &ops, false)?
            }
            t::ExprKind::Call { .. } => return self.unsupported(loc, "a call with `with`"),
            t::ExprKind::SysCall { name, args, .. } => self.sys_call(name, args, loc)?,
            t::ExprKind::Unary(op, a) => Expr::Unary(unop(*op), Box::new(self.expr(a)?)),
            t::ExprKind::Binary(op, a, b) => {
                let Some(bop) = binop(*op) else {
                    // `a -> b` is `!a || b`, `a <-> b` is `(a -> b) && (b -> a)` (IEEE 1800-2017
                    // 11.4.7)
                    let (a, b) = (self.expr(a)?, self.expr(b)?);
                    let imp = |x: &Expr, y: &Expr| Expr::Binary(BinOp::LOr, Box::new(Expr::Unary(UnOp::LNot, Box::new(x.clone()))), Box::new(y.clone()));
                    if *op == t::BinOp::LImpl {
                        return Ok(imp(&a, &b));
                    }
                    if super::stmt::side_effects(&a) || super::stmt::side_effects(&b) {
                        return self.unsupported(loc, "`<->` with an increment in an operand");
                    }
                    return Ok(Expr::Binary(BinOp::LAnd, Box::new(imp(&a, &b)), Box::new(imp(&b, &a))));
                };
                let a = self.expr(a)?;
                let b = self.expr(b)?;
                Expr::Binary(bop, Box::new(a), Box::new(b))
            }
            t::ExprKind::Cond(c, a, b) => {
                let c = self.expr(c)?;
                let a = self.expr(a)?;
                let b = self.expr(b)?;
                Expr::Cond(Box::new(c), Box::new(a), Box::new(b))
            }
            // min:typ:max: typ
            t::ExprKind::MinTypMax(_, ty, _) => self.expr(ty)?,
            t::ExprKind::Concat(v) if !v.is_empty() => {
                let mut out = Vec::new();
                for x in v {
                    out.push(self.expr(x)?);
                }
                Expr::Concat(out)
            }
            t::ExprKind::Repl(n, v) if !v.is_empty() => {
                let n = self.expr(n)?;
                let mut out = Vec::new();
                for x in v {
                    out.push(self.expr(x)?);
                }
                Expr::Repl(Box::new(n), out)
            }
            t::ExprKind::Inside(a, set) => {
                let a = self.expr(a)?;
                let mut acc: Option<Expr> = None;
                for r in set {
                    let x = self.set_item(&a, r)?;
                    acc = Some(match acc {
                        None => x,
                        Some(p) => Expr::Binary(BinOp::LOr, Box::new(p), Box::new(x)),
                    });
                }
                acc.unwrap()
            }
            t::ExprKind::Pattern { ty, items } => {
                let items = self.pattern(items, loc)?;
                let ty = match ty {
                    Some(dt) => Some(Rc::new(self.data_type_req(dt)?)),
                    None => None,
                };
                Expr::Pattern(ty, items)
            }
            t::ExprKind::Cast(target, x) => match target {
                t::CastTarget::Size(s) => {
                    let s = self.expr(s)?;
                    let x = self.expr(x)?;
                    cast(s, x, 2)
                }
                t::CastTarget::Signed => Expr::SysCall("$signed".into(), vec![self.expr(x)?]),
                t::CastTarget::Unsigned => Expr::SysCall("$unsigned".into(), vec![self.expr(x)?]),
                t::CastTarget::Const => return self.unsupported(loc, "a const cast"),
                t::CastTarget::Type(dt) => {
                    let string = match &dt.kind {
                        t::TypeKind::String => true,
                        t::TypeKind::Named(n) if n.scopes.is_empty() => self.types.get(&n.name).is_some_and(|t| t.kind == NetKind::String && t.unpacked.is_empty()),
                        _ => false,
                    };
                    if string {
                        return Ok(Expr::SysCall("$__tostring".into(), vec![self.expr(x)?]));
                    }
                    if matches!(dt.kind, t::TypeKind::Real(_)) {
                        return self.unsupported(loc, "a cast to a real type");
                    }
                    let t = Rc::new(self.data_type_req(dt)?);
                    let x = typed_patterns(self.expr(x)?, &t);
                    cast(t.width(), x, if t.signed { 1 } else { 0 })
                }
            },
            t::ExprKind::Type(dt) => Expr::Type(Rc::new(self.data_type_req(dt)?)),
            // `{}`: an empty queue
            t::ExprKind::Concat(_) => Expr::Concat(vec![]),
            t::ExprKind::Repl(..) => return self.unsupported(loc, "an empty replication"),
            t::ExprKind::Null => Expr::Null,
            // (the last index of a queue; elsewhere an error in elaboration)
            t::ExprKind::Dollar => Expr::Last,
            t::ExprKind::Step => return self.unsupported(loc, "1step"),
            t::ExprKind::Default => return self.unsupported(loc, "`default` here"),
            t::ExprKind::Root => return self.unsupported(loc, "$root alone"),
            t::ExprKind::IncDec { prefix, inc, e } => Expr::IncDec { prefix: *prefix, inc: *inc, e: Box::new(self.lvalue(e)?) },
            t::ExprKind::Assign(..) => return self.unsupported(loc, "an assignment in an expression"),
            t::ExprKind::Dist(..) => return self.unsupported(loc, "dist"),
            t::ExprKind::Matches(..) | t::ExprKind::TripleAnd(..) => return self.unsupported(loc, "pattern matching"),
            t::ExprKind::Stream { .. } => return self.unsupported(loc, "a streaming concatenation"),
            t::ExprKind::Tagged(..) => return self.unsupported(loc, "a tagged union expression"),
            // Verilog code may name a variable `new`
            t::ExprKind::New { size: None, args: None, copy: None } if self.vars.contains_key("new") => Expr::Ident(self.name("new", loc)),
            t::ExprKind::New { size: Some(n), args, copy: None } => {
                let n = self.expr(n)?;
                let init = match args.as_deref() {
                    None => None,
                    Some([t::Arg::Pos(Some(x))]) => Some(Box::new(self.expr(x)?)),
                    Some(_) => return self.unsupported(loc, "these arguments of new[]"),
                };
                Expr::NewArray(Box::new(n), init)
            }
            t::ExprKind::New { size: None, args, copy: None } => Expr::New(match args {
                Some(a) => self.call_args(a, loc)?,
                None => vec![],
            }),
            t::ExprKind::New { size: None, args: None, copy: Some(x) } => Expr::NewCopy(Box::new(self.expr(x)?)),
            t::ExprKind::New { .. } => return self.unsupported(loc, "new"),
            t::ExprKind::TypedNew { class, args } => {
                if !class.scopes.is_empty() || class.params.is_some() {
                    return self.unsupported(loc, "a constructor call of this class type");
                }
                let c = if self.classes.contains_key(&class.name) {
                    class.name.clone()
                } else {
                    match self.types.get(&class.name).and_then(|t| t.class.clone()) {
                        Some(c) => c,
                        None => return self.err(loc, format!("'{}' is not a class", class.name)),
                    }
                };
                let args = match args {
                    Some(a) => self.call_args(a, loc)?,
                    None => Vec::new(),
                };
                Expr::NewTyped(c, args)
            }
            t::ExprKind::Prop(_) => return self.unsupported(loc, "a property expression"),
        })
    }

    /// An assignment target.
    pub(crate) fn lvalue(&mut self, e: &t::Expr) -> Result<Expr> {
        if let t::ExprKind::Concat(v) = &e.kind {
            let mut out = Vec::new();
            for x in v {
                out.push(self.lvalue(x)?);
            }
            return Ok(Expr::Concat(out));
        }
        let (b, ops) = flatten(e);
        if ops.iter().any(|o| matches!(o, Op::Call(..))) {
            return self.unsupported(e.loc, "a call as an assignment target");
        }
        match b {
            Base::Name(..) => self.chain(b, &ops, true),
            Base::Other(t::Expr { kind: t::ExprKind::New { size: None, args: None, copy: None }, loc }) if self.vars.contains_key("new") => {
                self.chain(Base::Name("new", *loc), &ops, true)
            }
            // `C::p = ...`: a static property
            Base::Other(t::Expr { kind: t::ExprKind::Scoped(n), loc }) if n.scopes.len() == 1 && n.params.is_none() && n.scopes[0].params.is_none() => {
                match self.static_prop(&Expr::Ident(n.scopes[0].name.clone()), &n.name) {
                    Some(v) => self.selects(Expr::Ident(v), &ops),
                    // `P::v = ...`: a variable of a package
                    None if self.packages.contains_key(&n.scopes[0].name) => self.chain(b, &ops, true),
                    None => self.unsupported(*loc, "this assignment target"),
                }
            }
            Base::Other(x) => self.unsupported(x.loc, "this assignment target"),
        }
    }

    /// One item of an `inside` set: `[lo:hi]` (a `$` bound is open) or a value compared with
    /// `==?`; the test of `e` against it.
    pub(crate) fn set_item(&mut self, e: &Expr, r: &t::ValueRange) -> Result<Expr> {
        let b = |x: &Expr| Box::new(x.clone());
        match r {
            t::ValueRange::Range(lo, hi) => {
                let lo = if matches!(lo.kind, t::ExprKind::Dollar) { None } else { Some(self.expr(lo)?) };
                let hi = if matches!(hi.kind, t::ExprKind::Dollar) { None } else { Some(self.expr(hi)?) };
                let ge = lo.map(|lo| Expr::Binary(BinOp::Ge, b(e), Box::new(lo)));
                let le = hi.map(|hi| Expr::Binary(BinOp::Le, b(e), Box::new(hi)));
                Ok(match (ge, le) {
                    (Some(g), Some(l)) => Expr::Binary(BinOp::LAnd, Box::new(g), Box::new(l)),
                    (Some(t), None) | (None, Some(t)) => t,
                    (None, None) => Expr::Num(crate::ast::dec(1)),
                })
            }
            t::ValueRange::Value(v) => {
                let v = self.expr(v)?;
                Ok(Expr::Binary(BinOp::WildEq, b(e), Box::new(v)))
            }
        }
    }

    /// The items of `'{...}` (resolved in elaboration against the target type).
    fn pattern(&mut self, items: &t::PatternItems, loc: t::Loc) -> Result<Vec<PatItem>> {
        let mut out = Vec::new();
        match items {
            t::PatternItems::Positional(v) => {
                for x in v {
                    out.push(PatItem::Pos(self.expr(x)?));
                }
            }
            t::PatternItems::Keyed(v) => {
                for (k, x) in v {
                    let x = self.expr(x)?;
                    out.push(match &k.kind {
                        t::ExprKind::Ident(n) => PatItem::Key(n.clone(), x),
                        t::ExprKind::Default => PatItem::Default(x),
                        _ => return self.unsupported(k.loc, "this assignment pattern key"),
                    });
                }
            }
            t::PatternItems::Repl(..) => return self.unsupported(loc, "a replication in an assignment pattern"),
        }
        Ok(out)
    }

    /// Positional arguments of a call.
    pub(crate) fn call_args(&mut self, args: &[t::Arg], loc: t::Loc) -> Result<Vec<Expr>> {
        let mut v = Vec::new();
        for a in args {
            match a {
                t::Arg::Pos(Some(e)) => v.push(self.arg(e)?),
                t::Arg::Pos(None) => v.push(Expr::EmptyArg),
                t::Arg::Named(..) => return self.unsupported(loc, "a named argument"),
            }
        }
        Ok(v)
    }

    /// An argument: a bare type keyword or type name the parser took for a type is a name
    /// there (`$display(bit)` with a variable `bit`), except for `$bits`.
    pub(crate) fn arg(&mut self, e: &t::Expr) -> Result<Expr> {
        if let t::ExprKind::Type(dt) = &e.kind {
            if dt.signed.is_none() && dt.packed.is_empty() {
                let n = match &dt.kind {
                    t::TypeKind::Int(k) => Some(super::types::int_name(*k).to_string()),
                    t::TypeKind::String => Some("string".to_string()),
                    t::TypeKind::Named(n) if n.scopes.is_empty() && n.params.is_none() => Some(n.name.clone()),
                    _ => None,
                };
                if let Some(n) = n {
                    return self.expr(&t::Expr::new(t::ExprKind::Ident(n), e.loc));
                }
            }
        }
        self.expr(e)
    }

    fn sys_call(&mut self, name: &str, args: &[t::Arg], loc: t::Loc) -> Result<Expr> {
        if name == "$bits" {
            if let Some(t::Arg::Pos(Some(t::Expr { kind: t::ExprKind::Type(dt), .. }))) = args.first() {
                return Ok(self.data_type_req(dt)?.bits());
            }
        }
        if ARRAY_QUERIES.contains(&name) && (1..=2).contains(&args.len()) {
            if let t::Arg::Pos(Some(t::Expr { kind: t::ExprKind::Ident(n), loc: nloc })) = &args[0] {
                // (a queue or dynamic array: its size is known at run time, in the compiler)
                if self.vars.get(n).is_some_and(|v| v.0.heap.is_none()) {
                    let dim = match args.get(1) {
                        Some(t::Arg::Pos(Some(e))) => Some(e),
                        None => None,
                        _ => return self.unsupported(loc, "this argument"),
                    };
                    return self.array_query(name, n, dim, *nloc);
                }
            }
        }
        let args = self.call_args(args, loc)?;
        Ok(Expr::SysCall(name.to_string(), args))
    }

    /// Selects and members after a variable whose type has members.
    fn members(&mut self, mut e: Expr, ops: &[Op]) -> Result<Expr> {
        for (k, op) in ops.iter().enumerate() {
            e = match op {
                Op::Member(m, _) => Expr::Member(Box::new(e), m.to_string()),
                _ => self.selects(e, &ops[k..k + 1])?,
            };
        }
        Ok(e)
    }

    /// Members, method calls and selects after a class handle.
    fn class_chain(&mut self, mut e: Expr, ops: &[Op], lvalue: bool) -> Result<Expr> {
        let mut k = 0;
        while k < ops.len() {
            e = match (&ops[k], ops.get(k + 1)) {
                (Op::Member(m, _), Some(Op::Call(a, l))) => {
                    if lvalue {
                        return self.unsupported(*l, "a method call as an assignment target");
                    }
                    let args = self.call_args(a, *l)?;
                    k += 1;
                    Expr::MethodCall(Box::new(e), m.to_string(), args)
                }
                // `this.super` is `super`
                (Op::Member(m, _), _) if *m == "super" && matches!(&e, Expr::Ident(n) if n == "this") => Expr::Ident("super".into()),
                (Op::Member(m, _), _) => match self.static_prop(&e, m) {
                    Some(v) => Expr::Ident(v),
                    None => Expr::Member(Box::new(e), m.to_string()),
                },
                (Op::Call(_, l), _) => return self.unsupported(*l, "a call here"),
                _ => self.selects(e, &ops[k..k + 1])?,
            };
            k += 1;
        }
        Ok(e)
    }

    /// Generic selects after a name.
    fn selects(&mut self, mut e: Expr, ops: &[Op]) -> Result<Expr> {
        for op in ops {
            e = match op {
                Op::Index(i) => Expr::Index(Box::new(e), Box::new(self.expr(i)?)),
                Op::Range(k, a, b) => {
                    let a = self.expr(a)?;
                    let b = self.expr(b)?;
                    match k {
                        t::RangeKind::Colon => Expr::Range(Box::new(e), Box::new(a), Box::new(b)),
                        t::RangeKind::Plus => Expr::IndexedRange(Box::new(e), Box::new(a), Box::new(b), true),
                        t::RangeKind::Minus => Expr::IndexedRange(Box::new(e), Box::new(a), Box::new(b), false),
                    }
                }
                Op::Member(m, loc) => return self.unsupported(*loc, &format!("the member '.{m}' here")),
                Op::Call(_, loc) => return self.unsupported(*loc, "a call here"),
            };
        }
        Ok(e)
    }

    /// The hierarchical part of a chain: `a.b[2].c` -> parts, with `name[N]` / `name[@N]`
    /// segments; returns the parts and the number of ops used.
    fn hier_tail(&mut self, name: &str, ops: &[Op]) -> Result<(Vec<String>, usize)> {
        let mut parts = vec![name.to_string()];
        let mut k = 0;
        loop {
            if let (Some(Op::Index(i)), Some(Op::Member(..))) = (ops.get(k), ops.get(k + 1)) {
                let v = self.expr(i)?;
                let last = parts.last_mut().unwrap();
                *last = match lit_index(&v) {
                    Some(n) => format!("{last}[{n}]"),
                    None => format!("{last}[@{}]", push_hier_index(v)),
                };
                k += 1;
            }
            match ops.get(k) {
                Some(Op::Member(m, _)) => {
                    parts.push(m.to_string());
                    k += 1;
                }
                _ => break,
            }
        }
        Ok((parts, k))
    }

    /// A pure hierarchical name (`a.b[1].c`) as its parts; None for anything else.
    pub(crate) fn hier_path(&mut self, e: &t::Expr) -> Result<Option<Vec<String>>> {
        let (b, ops) = flatten(e);
        let Base::Name(n, _) = b else { return Ok(None) };
        let (parts, k) = self.hier_tail(n, &ops)?;
        Ok((k == ops.len()).then_some(parts))
    }

    fn chain(&mut self, b: Base, ops: &[Op], lvalue: bool) -> Result<Expr> {
        let (name, loc) = match b {
            Base::Other(e) => {
                let t::ExprKind::Scoped(n) = &e.kind else {
                    if ops.is_empty() {
                        return self.expr(e);
                    }
                    return self.unsupported(e.loc, "a select of this expression");
                };
                if n.scopes.len() != 1 || n.params.is_some() || n.scopes[0].params.is_some() || n.scopes[0].name == "$unit" {
                    return self.unsupported(e.loc, "this scoped name");
                }
                let mut p = n.scopes[0].name.clone();
                // `P::v = x`, `P::v[i] = x`, `P::s.m = x`: a variable of package P
                if lvalue {
                    let Some(mut pk) = self.packages.get(&p).cloned() else {
                        return self.err(e.loc, format!("unknown package '{p}'"));
                    };
                    if let Some(o) = pk.exports.get(&n.name) {
                        p = o.clone();
                        pk = self.packages[&p].clone();
                    }
                    let Some(v) = pk.vars.get(&n.name) else {
                        return self.unsupported(e.loc, "this scoped name as an assignment target");
                    };
                    if v.0.class.is_some() && !ops.is_empty() {
                        return self.unsupported(e.loc, "a property of a package's class handle as an assignment target");
                    }
                    let has_struct = v.0.has_struct();
                    self.use_package(&p);
                    let base = Expr::Hier(vec![p, n.name.clone()]);
                    return if has_struct { self.members(base, ops) } else { self.selects(base, ops) };
                }
                // `C::p`: a static property of class C
                if let Some(v) = self.static_prop(&Expr::Ident(p.clone()), &n.name) {
                    return self.selects(Expr::Ident(v), ops);
                }
                // `C::f(args)`: a static method of class C
                if self.classes.get(&p).is_some_and(|c| c.methods.contains(&n.name)) && n.name != "new" {
                    let (args, rest) = match ops.split_first() {
                        Some((Op::Call(a, l), rest)) => (self.call_args(a, *l)?, rest),
                        _ => (vec![], ops),
                    };
                    if !rest.is_empty() {
                        return self.unsupported(e.loc, "a select of a call");
                    }
                    return Ok(Expr::StaticCall(p, n.name.clone(), args));
                }
                // `C::P`: a parameter or enum constant of class C (in its scope)
                if self.classes.get(&p).is_some_and(|c| c.consts.contains(&n.name)) && !matches!(ops.first(), Some(Op::Call(..))) {
                    return self.selects(Expr::Ident(format!("{p}::{}", n.name)), ops);
                }
                let Some(mut pk) = self.packages.get(&p).cloned() else {
                    return self.err(e.loc, format!("unknown package '{p}'"));
                };
                // a name the package exports: the package that declares it
                if let Some(o) = pk.exports.get(&n.name) {
                    p = o.clone();
                    pk = self.packages[&p].clone();
                }
                let call = matches!(ops.first(), Some(Op::Call(..)));
                if let Some(v) = pk.vars.get(&n.name).filter(|_| !call) {
                    let t = &v.0;
                    if !matches!(ops.first(), Some(Op::Member(..))) || !(t.has_struct() || t.enum_names.is_some() || t.class.is_some()) {
                        return self.selects(Expr::Hier(vec![p, n.name.clone()]), ops);
                    }
                    if t.has_struct() {
                        return self.members(Expr::Hier(vec![p, n.name.clone()]), ops);
                    }
                    // `P::v.m`: members and methods as for a variable of this scope, under the
                    // name `P::v` (its enum constants are the package's)
                    let mut v = v.clone();
                    if let Some(names) = &t.enum_names {
                        let q = names.iter().map(|c| if c.contains("::") { c.clone() } else { format!("{p}::{c}") }).collect();
                        Rc::make_mut(&mut v.0).enum_names = Some(Rc::new(q));
                    }
                    self.use_package(&p);
                    let name = format!("{p}::{}", n.name);
                    self.vars.insert(name.clone(), v);
                    return self.chain(Base::Name(&name, e.loc), ops, lvalue);
                }
                if pk.funcs.contains(&n.name) {
                    self.use_package(&p);
                    let (args, rest) = match ops.split_first() {
                        Some((Op::Call(a, l), rest)) => (self.call_args(a, *l)?, rest),
                        _ => (vec![], ops),
                    };
                    if !rest.is_empty() {
                        return self.unsupported(e.loc, "a select of a call");
                    }
                    return Ok(Expr::Call(format!("{p}.{}", n.name), args));
                }
                self.use_package(&p);
                if let Some(t) = pk.types.get(&n.name) {
                    self.types.insert(n.name.clone(), t.clone());
                    (n.name.clone(), e.loc)
                } else {
                    // a parameter (or enum constant) of the package: looked up in its scope,
                    // not through the imports (another package may have the same name)
                    (format!("{p}::{}", n.name), e.loc)
                }
            }
            Base::Name(n @ ("this" | "super"), loc) => (n.to_string(), loc),
            Base::Name(n, loc) => (self.name(n, loc), loc),
        };
        let _ = loc;
        if self.is_class_var(&name) {
            // `f()` of a function returning a class handle: a call (`f().m` a member of it)
            if let Some((Op::Call(a, l), rest)) = ops.split_first() {
                let args = self.call_args(a, *l)?;
                return self.class_chain(Expr::Call(name, args), rest, lvalue);
            }
            return self.class_chain(Expr::Ident(name), ops, lvalue);
        }
        let var = self.vars.get(&name).map(|v| v.0.clone());
        // a variable whose type has members (a struct, or a type parameter): members are
        // resolved in elaboration
        if var.as_ref().is_some_and(|t| t.has_struct() || t.param.is_some()) && !matches!(ops.first(), Some(Op::Call(..))) {
            return self.members(Expr::Ident(name), ops);
        }
        if lvalue {
            let (parts, k) = self.hier_tail(&name, ops)?;
            let base = if parts.len() == 1 { Expr::Ident(name) } else { Expr::Hier(parts) };
            return self.selects(base, &ops[k..]);
        }
        if let (Some(t), Some(Op::Member(m, mloc))) = (&var, ops.first()) {
            if t.enum_names.is_some() && ENUM_METHODS.contains(m) {
                let (arg, used) = match ops.get(1) {
                    Some(Op::Call(a, l)) => match a {
                        [] => (None, 2),
                        [t::Arg::Pos(Some(x))] => (Some(x), 2),
                        _ => return self.unsupported(*l, "these enum method arguments"),
                    },
                    _ => (None, 1),
                };
                if ops.len() > used {
                    return self.unsupported(*mloc, "a select of an enum method");
                }
                return self.enum_method(name, m, arg, *mloc);
            }
        }
        if let Some((Op::Call(a, l), rest)) = ops.split_first() {
            let args = self.call_args(a, *l)?;
            if !rest.is_empty() {
                return self.unsupported(*l, "a select of a call");
            }
            return Ok(Expr::Call(name, args));
        }
        let (parts, k) = self.hier_tail(&name, ops)?;
        if parts.len() > 1 {
            if let Some((Op::Call(a, l), rest)) = ops[k..].split_first() {
                // hierarchical function call: inst.f(args)
                let args = self.call_args(a, *l)?;
                if !rest.is_empty() {
                    return self.unsupported(*l, "a select of a call");
                }
                return Ok(Expr::Call(parts.join("."), args));
            }
            return self.selects(Expr::Hier(parts), &ops[k..]);
        }
        self.selects(Expr::Ident(name), &ops[k..])
    }
}

/// `T'(e)` of an assignment pattern: the pattern gets type `T` (also in a conditional's
/// branches).
fn typed_patterns(e: Expr, t: &Rc<super::SvType>) -> Expr {
    match e {
        Expr::Pattern(None, items) => Expr::Pattern(Some(t.clone()), items),
        Expr::Cond(c, a, b) => Expr::Cond(c, Box::new(typed_patterns(*a, t)), Box::new(typed_patterns(*b, t))),
        e => e,
    }
}
