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
    match &e.kind {
        ExprKind::Num(n) if !n.digits.contains(['x', 'z', 'X', 'Z', '?']) => i64::from_str_radix(&n.digits.replace('_', ""), n.base).ok(),
        ExprKind::Unary(UnOp::Neg, b) => match &b.kind {
            ExprKind::Num(n) => i64::from_str_radix(&n.digits.replace('_', ""), n.base).ok().map(|v| -v),
            _ => None,
        },
        _ => None,
    }
}

impl Lower {
    /// An expression, with its line (and that of the parts made for it).
    pub(crate) fn expr(&mut self, e: &t::Expr) -> Result<Expr> {
        let mut x = self.expr_at(e)?;
        x.fill_line(e.loc.line);
        Ok(x)
    }

    fn expr_at(&mut self, e: &t::Expr) -> Result<Expr> {
        let loc = e.loc;
        Ok(match &e.kind {
            t::ExprKind::Num(n) => ExprKind::Num(n.clone()).into(),
            t::ExprKind::Real(r) => ExprKind::Real(*r).into(),
            t::ExprKind::Time(v, u) => self.time_literal(*v, *u),
            t::ExprKind::Str(s) => ExprKind::Str(s.clone()).into(),
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
            t::ExprKind::Call { func, args, with_, .. } if Self::randomize_form(func).is_some() => self.randomize(func, args, with_.as_deref(), loc)?,
            t::ExprKind::Call { with_: None, parens: true, .. } => {
                let (b, ops) = flatten(e);
                self.chain(b, &ops, false)?
            }
            t::ExprKind::Call { with_: None, parens: false, .. } => return self.unsupported(loc, "this call without parentheses"),
            t::ExprKind::Call { func, args, with_: Some(w), .. } => {
                let (t::With::Expr(we), t::ExprKind::Member(base, m)) = (&**w, &func.kind) else {
                    return self.unsupported(loc, "a call with `with` that is not an array method");
                };
                // (the optional argument names the iterator, IEEE 1800-2017 7.12)
                let iter = match args.as_slice() {
                    [] => "item".to_string(),
                    [t::Arg::Pos(Some(t::Expr { kind: t::ExprKind::Ident(n), .. }))] => n.clone(),
                    _ => return self.err(loc, format!("the argument of '{m}' with `with` is the name of its iterator")),
                };
                let arr = self.expr(base)?;
                let depth = self.iters.len();
                // the iterator's type, for its members and methods: the element type of an
                // array variable (otherwise none: members are resolved in elaboration)
                let it = format!("$it{depth}");
                let elem = match &arr.kind {
                    ExprKind::Ident(n) => match self.vars.get(n) {
                        // a queue or dynamic array (`heap`), a fixed-size array (one dimension)
                        Some((t, u)) if t.heap.is_some() && u.is_empty() => {
                            let mut t = (**t).clone();
                            t.heap = None;
                            Some((Rc::new(t), vec![]))
                        }
                        Some((t, u)) if t.heap.is_none() && u.len() == 1 => Some((t.clone(), vec![])),
                        _ => None,
                    },
                    _ => None,
                };
                let saved = match elem {
                    Some(t) => self.vars.insert(it.clone(), t),
                    None => self.vars.remove(&it),
                };
                self.iters.push(iter);
                let with = self.expr(we);
                self.iters.pop();
                match saved {
                    Some(v) => self.vars.insert(it, v),
                    None => self.vars.remove(&it),
                };
                ExprKind::ArrayWith { arr: Box::new(arr), method: m.clone(), depth, with: Box::new(with?) }.into()
            }
            t::ExprKind::SysCall { name, args, .. } => self.sys_call(name, args, loc)?,
            t::ExprKind::Unary(op, a) => ExprKind::Unary(unop(*op), Box::new(self.expr(a)?)).into(),
            t::ExprKind::Binary(op, a, b) => {
                let Some(bop) = binop(*op) else {
                    // `a -> b` is `!a || b`, `a <-> b` is `(a -> b) && (b -> a)` (IEEE 1800-2017
                    // 11.4.7)
                    let (a, b) = (self.expr(a)?, self.expr(b)?);
                    let imp = |x: &Expr, y: &Expr| ExprKind::Binary(BinOp::LOr, Box::new(ExprKind::Unary(UnOp::LNot, Box::new(x.clone())).into()), Box::new(y.clone()));
                    if *op == t::BinOp::LImpl {
                        return Ok(imp(&a, &b).into());
                    }
                    if super::stmt::side_effects(&a) || super::stmt::side_effects(&b) {
                        return self.unsupported(loc, "`<->` with an increment in an operand");
                    }
                    return Ok(ExprKind::Binary(BinOp::LAnd, Box::new(imp(&a, &b).into()), Box::new(imp(&b, &a).into())).into());
                };
                let a = self.expr(a)?;
                let b = self.expr(b)?;
                ExprKind::Binary(bop, Box::new(a), Box::new(b)).into()
            }
            t::ExprKind::Cond(c, a, b) => {
                let c = self.expr(c)?;
                let a = self.expr(a)?;
                let b = self.expr(b)?;
                ExprKind::Cond(Box::new(c), Box::new(a), Box::new(b)).into()
            }
            // min:typ:max: typ
            t::ExprKind::MinTypMax(_, ty, _) => self.expr(ty)?,
            t::ExprKind::Concat(v) if !v.is_empty() => {
                let mut out = Vec::new();
                for x in v {
                    out.push(self.expr(x)?);
                }
                ExprKind::Concat(out).into()
            }
            t::ExprKind::Repl(n, v) if !v.is_empty() => {
                let n = self.expr(n)?;
                let mut out = Vec::new();
                for x in v {
                    out.push(self.expr(x)?);
                }
                ExprKind::Repl(Box::new(n), out).into()
            }
            t::ExprKind::Inside(a, set) => {
                let a = self.expr(a)?;
                let mut acc: Option<Expr> = None;
                for r in set {
                    let x = self.set_item(&a, r)?;
                    acc = Some(match acc {
                        None => x,
                        Some(p) => ExprKind::Binary(BinOp::LOr, Box::new(p), Box::new(x)).into(),
                    });
                }
                acc.unwrap()
            }
            t::ExprKind::Pattern { ty, items } => {
                let items = self.pattern(items)?;
                let ty = match ty {
                    Some(dt) => Some(Rc::new(self.data_type_req(dt)?)),
                    None => None,
                };
                ExprKind::Pattern(ty, items).into()
            }
            t::ExprKind::Cast(target, x) => match target {
                t::CastTarget::Size(s) => {
                    let s = self.expr(s)?;
                    let x = self.expr(x)?;
                    cast(s, x, 2)
                }
                t::CastTarget::Signed => ExprKind::SysCall("$signed".into(), vec![self.expr(x)?]).into(),
                t::CastTarget::Unsigned => ExprKind::SysCall("$unsigned".into(), vec![self.expr(x)?]).into(),
                t::CastTarget::Const => return self.unsupported(loc, "a const cast"),
                t::CastTarget::Type(dt) => {
                    let string = match &dt.kind {
                        t::TypeKind::String => true,
                        t::TypeKind::Named(n) if n.scopes.is_empty() => self.types.get(&n.name).is_some_and(|t| t.kind == NetKind::String && t.unpacked.is_empty()),
                        _ => false,
                    };
                    if string {
                        return Ok(ExprKind::SysCall("$__tostring".into(), vec![self.expr(x)?]).into());
                    }
                    if matches!(dt.kind, t::TypeKind::Real(_)) {
                        return self.unsupported(loc, "a cast to a real type");
                    }
                    let t = Rc::new(self.data_type_req(dt)?);
                    let x = typed_patterns(self.expr(x)?, &t);
                    cast(t.width(), x, if t.signed { 1 } else { 0 })
                }
            },
            t::ExprKind::Type(dt) => ExprKind::Type(Rc::new(self.data_type_req(dt)?)).into(),
            // `{}`: an empty queue
            t::ExprKind::Concat(_) => ExprKind::Concat(vec![]).into(),
            t::ExprKind::Repl(..) => return self.unsupported(loc, "an empty replication"),
            t::ExprKind::Null => ExprKind::Null.into(),
            // (the last index of a queue; elsewhere an error in elaboration)
            t::ExprKind::Dollar => ExprKind::Last.into(),
            t::ExprKind::Step => return self.unsupported(loc, "1step"),
            t::ExprKind::Default => return self.unsupported(loc, "`default` here"),
            t::ExprKind::Root => return self.unsupported(loc, "$root alone"),
            t::ExprKind::IncDec { prefix, inc, e } => ExprKind::IncDec { prefix: *prefix, inc: *inc, e: Box::new(self.lvalue(e)?) }.into(),
            t::ExprKind::Assign(..) => return self.unsupported(loc, "an assignment in an expression"),
            t::ExprKind::Dist(..) => return self.unsupported(loc, "dist"),
            t::ExprKind::Matches(..) | t::ExprKind::TripleAnd(..) => return self.unsupported(loc, "pattern matching"),
            t::ExprKind::Stream { left, slice, items } => {
                let slice = match slice {
                    Some(s) => Some(Box::new(match &s.kind {
                        t::ExprKind::Type(dt) => ExprKind::Type(Rc::new(self.data_type_req(dt)?)).into(),
                        _ => self.expr(s)?,
                    })),
                    None => None,
                };
                let mut v = Vec::new();
                for (x, with) in items {
                    if with.is_some() {
                        return self.unsupported(loc, "`with` in a streaming concatenation");
                    }
                    v.push(self.expr(x)?);
                }
                ExprKind::Stream { left: *left, slice, items: v }.into()
            }
            t::ExprKind::Tagged(..) => return self.unsupported(loc, "a tagged union expression"),
            // Verilog code may name a variable `new`
            t::ExprKind::New { size: None, args: None, copy: None } if self.vars.contains_key("new") => ExprKind::Ident(self.name("new", loc)).into(),
            t::ExprKind::New { size: Some(n), args, copy: None } => {
                let n = self.expr(n)?;
                let init = match args.as_deref() {
                    None => None,
                    Some([t::Arg::Pos(Some(x))]) => Some(Box::new(self.expr(x)?)),
                    Some(_) => return self.unsupported(loc, "these arguments of new[]"),
                };
                ExprKind::NewArray(Box::new(n), init).into()
            }
            t::ExprKind::New { size: None, args, copy: None } => ExprKind::New(match args {
                Some(a) => self.call_args(a)?,
                None => vec![],
            }).into(),
            t::ExprKind::New { size: None, args: None, copy: Some(x) } => ExprKind::NewCopy(Box::new(self.expr(x)?)).into(),
            t::ExprKind::New { .. } => return self.unsupported(loc, "new"),
            t::ExprKind::TypedNew { class, args } => {
                // `C #(..)::new`: of a specialization
                let class = &self.despec(class, loc)?;
                if !class.scopes.is_empty() {
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
                    Some(a) => self.call_args(a)?,
                    None => Vec::new(),
                };
                ExprKind::NewTyped(c, args).into()
            }
            t::ExprKind::Prop(_) => return self.unsupported(loc, "a property expression"),
        })
    }

    /// An assignment target.
    pub(crate) fn lvalue(&mut self, e: &t::Expr) -> Result<Expr> {
        let mut x = self.lvalue_at(e)?;
        x.fill_line(e.loc.line);
        Ok(x)
    }

    fn lvalue_at(&mut self, e: &t::Expr) -> Result<Expr> {
        // `{>> {a, b}} = x`: the items are targets
        if let t::ExprKind::Stream { left, slice, items } = &e.kind {
            let slice = match slice {
                Some(s) => Some(Box::new(match &s.kind {
                    t::ExprKind::Type(dt) => ExprKind::Type(Rc::new(self.data_type_req(dt)?)).into(),
                    _ => self.expr(s)?,
                })),
                None => None,
            };
            let mut v = Vec::new();
            for (x, with) in items {
                if with.is_some() {
                    return self.unsupported(e.loc, "`with` in a streaming concatenation");
                }
                v.push(self.lvalue(x)?);
            }
            return Ok(ExprKind::Stream { left: *left, slice, items: v }.into());
        }
        if let t::ExprKind::Concat(v) = &e.kind {
            let mut out = Vec::new();
            for x in v {
                out.push(self.lvalue(x)?);
            }
            return Ok(ExprKind::Concat(out).into());
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
                match self.static_prop(&ExprKind::Ident(n.scopes[0].name.clone()).into(), &n.name) {
                    Some(v) => self.selects(ExprKind::Ident(v).into(), &ops),
                    // `P::v = ...`: a variable of a package
                    None if self.packages.contains_key(&n.scopes[0].name) => self.chain(b, &ops, true),
                    // `P::v = ...` inside package P
                    None if self.pkg_ctx.as_ref().is_some_and(|c| c.name == n.scopes[0].name) => self.chain(Base::Name(&n.name, *loc), &ops, true),
                    // of a class declared later (`typedef class C;`)
                    None if self.types.get(&n.scopes[0].name).is_some_and(|t| t.class.is_some()) => self.chain(b, &ops, true),
                    None => self.unsupported(*loc, "this assignment target"),
                }
            }
            // `C #(..)::p = ...`: a static property of a specialization
            Base::Other(t::Expr { kind: t::ExprKind::Scoped(n), .. }) if n.params.is_some() || n.scopes.iter().any(|s| s.params.is_some()) => self.chain(b, &ops, true),
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
                let ge = lo.map(|lo| ExprKind::Binary(BinOp::Ge, b(e), Box::new(lo)));
                let le = hi.map(|hi| ExprKind::Binary(BinOp::Le, b(e), Box::new(hi)));
                Ok(match (ge, le) {
                    (Some(g), Some(l)) => ExprKind::Binary(BinOp::LAnd, Box::new(g.into()), Box::new(l.into())).into(),
                    (Some(t), None) | (None, Some(t)) => t.into(),
                    (None, None) => ExprKind::Num(crate::ast::dec(1)).into(),
                })
            }
            t::ValueRange::Value(v) => {
                let v = self.expr(v)?;
                Ok(ExprKind::Binary(BinOp::WildEq, b(e), Box::new(v)).into())
            }
        }
    }

    /// The items of `'{...}` (resolved in elaboration against the target type).
    fn pattern(&mut self, items: &t::PatternItems) -> Result<Vec<PatItem>> {
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
                        t::ExprKind::Type(dt) => PatItem::Type(Rc::new(self.data_type_req(dt)?), x),
                        _ => PatItem::Index(self.expr(k)?, x),
                    });
                }
            }
            t::PatternItems::Repl(n, v) => {
                let n = self.expr(n)?;
                let mut items = Vec::new();
                for x in v {
                    items.push(self.expr(x)?);
                }
                out.push(PatItem::Repl(n, items));
            }
        }
        Ok(out)
    }

    /// Positional arguments of a call.
    pub(crate) fn call_args(&mut self, args: &[t::Arg]) -> Result<Vec<Expr>> {
        let mut v = Vec::new();
        for a in args {
            match a {
                t::Arg::Pos(Some(e)) => v.push(self.arg(e)?),
                t::Arg::Pos(None) => v.push(ExprKind::EmptyArg.into()),
                t::Arg::Named(n, e) => {
                    let e = match e {
                        Some(e) => self.arg(e)?,
                        None => ExprKind::EmptyArg.into(),
                    };
                    v.push(ExprKind::NamedArg(n.clone(), Box::new(e)).into());
                }
            }
        }
        Ok(v)
    }

    /// An argument: a bare type keyword or type name the parser took for a type is a name
    /// there (`$display(bit)` with a variable `bit`, Verilog-2005), except for `$bits`.
    pub(crate) fn arg(&mut self, e: &t::Expr) -> Result<Expr> {
        let mut x = self.arg_at(e)?;
        x.fill_line(e.loc.line);
        Ok(x)
    }

    fn arg_at(&mut self, e: &t::Expr) -> Result<Expr> {
        if let t::ExprKind::Type(dt) = &e.kind {
            // `C #(..)::P` (parsed as a type): a constant of a specialization, unless the class
            // declares a type of that name
            if let t::TypeKind::Named(n) = &dt.kind {
                if n.scopes.len() == 1 && n.scopes[0].params.is_some() && dt.signed.is_none() && dt.packed.is_empty() {
                    let n2 = self.despec(n, e.loc)?;
                    let is_type = self.classes.get(&n2.scopes[0].name).is_some_and(|c| c.types.contains_key(&n2.name));
                    if !is_type {
                        return self.expr(&t::Expr::new(t::ExprKind::Scoped(Box::new(n2)), e.loc));
                    }
                    let dt2 = t::DataType { kind: t::TypeKind::Named(Box::new(n2)), ..(**dt).clone() };
                    return self.expr(&t::Expr::new(t::ExprKind::Type(Box::new(dt2)), e.loc));
                }
            }
            if dt.signed.is_none() && dt.packed.is_empty() {
                // (a keyword only where such a variable exists)
                let var = |l: &Self, n: &str| l.vars.contains_key(n).then(|| n.to_string());
                let n = match &dt.kind {
                    t::TypeKind::Int(k) => var(self, super::types::int_name(*k)),
                    t::TypeKind::String => var(self, "string"),
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
        if args.iter().any(|a| matches!(a, t::Arg::Named(..))) {
            return self.unsupported(loc, "a named argument of a system function");
        }
        let args = self.call_args(args)?;
        Ok(ExprKind::SysCall(name.to_string(), args).into())
    }

    /// Selects and members after a variable whose type has members.
    fn members(&mut self, mut e: Expr, ops: &[Op]) -> Result<Expr> {
        for (k, op) in ops.iter().enumerate() {
            e = match op {
                Op::Member(m, _) => ExprKind::Member(Box::new(e), m.to_string()).into(),
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
                    let args = self.call_args(a)?;
                    k += 1;
                    ExprKind::MethodCall(Box::new(e), m.to_string(), args).into()
                }
                // `this.super` is `super`
                (Op::Member(m, _), _) if *m == "super" && matches!(&e.kind, ExprKind::Ident(n) if n == "this") => ExprKind::Ident("super".into()).into(),
                (Op::Member(m, _), _) => match self.static_prop(&e, m) {
                    Some(v) => ExprKind::Ident(v).into(),
                    None => ExprKind::Member(Box::new(e), m.to_string()).into(),
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
                Op::Index(i) => ExprKind::Index(Box::new(e), Box::new(self.expr(i)?)).into(),
                Op::Range(k, a, b) => {
                    let a = self.expr(a)?;
                    let b = self.expr(b)?;
                    match k {
                        t::RangeKind::Colon => ExprKind::Range(Box::new(e), Box::new(a), Box::new(b)).into(),
                        t::RangeKind::Plus => ExprKind::IndexedRange(Box::new(e), Box::new(a), Box::new(b), true).into(),
                        t::RangeKind::Minus => ExprKind::IndexedRange(Box::new(e), Box::new(a), Box::new(b), false).into(),
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
        // the iterator of an array method's `with` expression: `item`, `item.index`
        // in `obj.randomize() with {...}`: a property of obj (unless a local name hides it)
        if let (Base::Name(n, nloc), Some((o, names))) = (b, &self.with_obj) {
            if (names.props.contains(n) || names.methods.contains(n)) && !self.wild.locals.iter().any(|l| l == n) {
                let o = o.clone();
                let mut v = vec![Op::Member(n, nloc)];
                v.extend(ops.iter().cloned());
                return self.class_chain(o, &v, lvalue);
            }
        }
        // a formal argument of a named sequence or property: the actual one, lowered where
        // the instance is
        if let Base::Name(n, _) = b {
            if let Some(a) = self.sva_actual(n) {
                let base = self.sva_caller(|l| l.expr(&a))?;
                return if ops.iter().any(|o| matches!(o, Op::Member(..))) { self.members(base, ops) } else { self.selects(base, ops) };
            }
        }
        // `local::x` there: the caller's x
        if let Base::Other(t::Expr { kind: t::ExprKind::Scoped(n), loc: sloc }) = b {
            if n.scopes.len() == 1 && n.scopes[0].name == "local" && n.scopes[0].params.is_none() && n.params.is_none() {
                let saved = self.with_obj.take();
                let r = self.chain(Base::Name(&n.name, *sloc), ops, lvalue);
                self.with_obj = saved;
                return r;
            }
        }
        // (the element is a variable `$it<k>` of the element's type, see `ArrayWith`)
        let mut iter = None;
        if let Base::Name(n, _) = b {
            if let Some(k) = self.iters.iter().rposition(|x| x == n) {
                if let Some(Op::Member("index", _)) = ops.first() {
                    let rest = match &ops[1..] {
                        [Op::Call(a, _), rest @ ..] if a.is_empty() => rest,
                        rest => rest,
                    };
                    return self.selects(ExprKind::Ident(format!("$ix{k}")).into(), rest);
                }
                iter = Some(format!("$it{k}"));
            }
        }
        let (name, loc) = match b {
            Base::Other(e) => {
                let t::ExprKind::Scoped(n) = &e.kind else {
                    if ops.is_empty() {
                        return self.expr(e);
                    }
                    return self.unsupported(e.loc, "a select of this expression");
                };
                // `C #(..)::x`: of a specialization
                let despecced;
                let n = if n.params.is_some() || n.scopes.iter().any(|s| s.params.is_some()) {
                    despecced = self.despec(n, e.loc)?;
                    &despecced
                } else {
                    n
                };
                // `C::T::f(args)`, T a typedef of class C naming a class (also of a class C
                // declared later, `typedef class C;`): a static method of that class, which the
                // compiler finds in C's scope
                if let ([c, td], Some((Op::Call(a, _), rest))) = (n.scopes.as_slice(), ops.split_first()) {
                    let class = match self.classes.get(&c.name) {
                        Some(cn) => cn.types.get(&td.name).filter(|t| t.class.is_some()).map(|_| c.name.clone()),
                        None => self.types.get(&c.name).and_then(|t| t.class.clone()),
                    };
                    if let Some(class) = class.filter(|_| c.params.is_none() && td.params.is_none() && n.params.is_none()) {
                        if !rest.is_empty() {
                            return self.unsupported(e.loc, "a select of a call");
                        }
                        let args = self.call_args(a)?;
                        return Ok(ExprKind::StaticCall(format!("{class}::{}", td.name), n.name.clone(), args).into());
                    }
                }
                // `C::T::x`, T a typedef of class C naming class D: `D::x`
                let collapsed;
                let n = if n.scopes.len() > 1 {
                    collapsed = self.class_typedef_scopes(n);
                    &collapsed
                } else {
                    n
                };
                if n.scopes.len() != 1 || n.params.is_some() || n.scopes[0].params.is_some() || n.scopes[0].name == "$unit" {
                    return self.unsupported(e.loc, "this scoped name");
                }
                let mut p = n.scopes[0].name.clone();
                // `P::x` inside package P: its own name x
                if self.pkg_ctx.as_ref().is_some_and(|c| c.name == p) && !self.classes.contains_key(&p) {
                    return self.chain(Base::Name(&n.name, e.loc), ops, lvalue);
                }
                // through a type parameter (a class per specialization): resolved by the
                // compiler
                if !self.classes.contains_key(&p) && !self.packages.contains_key(&p) && self.types.get(&p).is_some_and(|t| t.param.is_some()) {
                    if let Some((Op::Call(a, _), rest)) = ops.split_first() {
                        if !rest.is_empty() {
                            return self.unsupported(e.loc, "a select of a call");
                        }
                        let args = self.call_args(a)?;
                        return Ok(ExprKind::StaticCall(p, n.name.clone(), args).into());
                    }
                    let m: Expr = ExprKind::ClassMember { class: p, name: n.name.clone() }.into();
                    return if ops.iter().any(|o| matches!(o, Op::Member(..))) { self.class_chain(m, ops, lvalue) } else { self.selects(m, ops) };
                }
                // a class declared later (`typedef class C;`, its body in the same scope, IEEE
                // 1800-2017 8.27): `C::f(args)` a static method, `C::x` a static property
                if !self.classes.contains_key(&p) && !self.packages.contains_key(&p) {
                    if let Some(c) = self.types.get(&p).and_then(|t| t.class.clone()) {
                        if let Some((Op::Call(a, _), rest)) = ops.split_first() {
                            if !rest.is_empty() {
                                return self.unsupported(e.loc, "a select of a call");
                            }
                            let args = self.call_args(a)?;
                            return Ok(ExprKind::StaticCall(c, n.name.clone(), args).into());
                        }
                        return self.selects(ExprKind::Ident(format!("{c}${}", n.name)).into(), ops);
                    }
                }
                // `C #(..)::x`: a static property of a specialization, in its scope (reached
                // like a package's variable)
                if self.classes.get(&p).is_some_and(|c| c.spec_statics.contains(&n.name)) && !matches!(ops.first(), Some(Op::Call(..))) {
                    let m: Expr = ExprKind::Hier(vec![p, n.name.clone()]).into();
                    return if ops.iter().any(|o| matches!(o, Op::Member(..))) { self.class_chain(m, ops, lvalue) } else { self.selects(m, ops) };
                }
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
                    let base = ExprKind::Hier(vec![p, n.name.clone()]);
                    return if has_struct { self.members(base.into(), ops) } else { self.selects(base.into(), ops) };
                }
                self.check_unspecialized(&p, e.loc)?;
                // `C::p`: a static property of class C
                if let Some(v) = self.static_prop(&ExprKind::Ident(p.clone()).into(), &n.name) {
                    return self.selects(ExprKind::Ident(v).into(), ops);
                }
                // `C::f(args)`: a static method of class C
                if self.classes.get(&p).is_some_and(|c| c.methods.contains(&n.name)) && n.name != "new" {
                    let (args, rest) = match ops.split_first() {
                        Some((Op::Call(a, _), rest)) => (self.call_args(a)?, rest),
                        _ => (vec![], ops),
                    };
                    if !rest.is_empty() {
                        return self.unsupported(e.loc, "a select of a call");
                    }
                    return Ok(ExprKind::StaticCall(p, n.name.clone(), args).into());
                }
                // `C::P`: a parameter or enum constant of class C (in its scope)
                if self.classes.get(&p).is_some_and(|c| c.consts.contains(&n.name)) && !matches!(ops.first(), Some(Op::Call(..))) {
                    return self.selects(ExprKind::Ident(format!("{p}::{}", n.name)).into(), ops);
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
                    // `P::q.size()`: a method of a queue or an array (as `q.size()` here)
                    if let [Op::Member(m, _), Op::Call(a, _), rest @ ..] = ops {
                        if (t.heap.is_some() || !v.1.is_empty()) && rest.is_empty() {
                            let args = self.call_args(a)?;
                            self.use_package(&p);
                            return Ok(ExprKind::Call(format!("{p}.{}.{m}", n.name), args).into());
                        }
                    }
                    if !matches!(ops.first(), Some(Op::Member(..))) || !(t.has_struct() || t.enum_names.is_some() || t.class.is_some()) {
                        return self.selects(ExprKind::Hier(vec![p, n.name.clone()]).into(), ops);
                    }
                    if t.has_struct() {
                        return self.members(ExprKind::Hier(vec![p, n.name.clone()]).into(), ops);
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
                        Some((Op::Call(a, _), rest)) => (self.call_args(a)?, rest),
                        _ => (vec![], ops),
                    };
                    if !rest.is_empty() {
                        return self.unsupported(e.loc, "a select of a call");
                    }
                    return Ok(ExprKind::Call(format!("{p}.{}", n.name), args).into());
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
            Base::Name(_, loc) if iter.is_some() => (iter.unwrap(), loc),
            Base::Name(n @ ("this" | "super"), loc) => (n.to_string(), loc),
            Base::Name(n, loc) => {
                // (a call may name a subroutine declared later in the module)
                if !matches!(ops.first(), Some(Op::Call(..))) {
                    self.wild_ref(n, loc)?;
                }
                (self.name(n, loc), loc)
            }
        };
        let _ = loc;
        if self.is_class_var(&name) {
            // `f()` of a function returning a class handle: a call (`f().m` a member of it)
            if let Some((Op::Call(a, _), rest)) = ops.split_first() {
                let args = self.call_args(a)?;
                return self.class_chain(ExprKind::Call(name, args).into(), rest, lvalue);
            }
            return self.class_chain(ExprKind::Ident(name).into(), ops, lvalue);
        }
        let var = self.vars.get(&name).map(|v| v.0.clone());
        // a variable whose type has members (a struct, or a type parameter): members are
        // resolved in elaboration
        // (a queue's method call, `q.size()`, is not a member)
        let queue_call = var.as_ref().is_some_and(|t| t.heap.is_some()) && ops.iter().any(|o| matches!(o, Op::Call(..)));
        if var.as_ref().is_some_and(|t| t.has_struct() || t.param.is_some()) && !matches!(ops.first(), Some(Op::Call(..))) && !queue_call {
            // (of a type parameter: also method calls, of a class or an enum)
            if var.as_ref().is_some_and(|t| t.param.is_some() && !t.has_struct()) && ops.iter().any(|o| matches!(o, Op::Call(..))) {
                return self.class_chain(ExprKind::Ident(name).into(), ops, lvalue);
            }
            return self.members(ExprKind::Ident(name).into(), ops);
        }
        if lvalue {
            let (parts, k) = self.hier_tail(&name, ops)?;
            let base = if parts.len() == 1 { ExprKind::Ident(name) } else { ExprKind::Hier(parts) };
            return self.selects(base.into(), &ops[k..]);
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
            let args = self.call_args(a)?;
            let call: Expr = ExprKind::Call(name, args).into();
            // `f().m()`, `f().p`: through the handle the function returns
            if matches!(rest.first(), Some(Op::Member(..))) {
                return self.class_chain(call, rest, lvalue);
            }
            if !rest.is_empty() {
                return self.unsupported(*l, "a select of a call");
            }
            return Ok(call);
        }
        let (parts, k) = self.hier_tail(&name, ops)?;
        if parts.len() > 1 {
            if let Some((Op::Call(a, l), rest)) = ops[k..].split_first() {
                // hierarchical function call: inst.f(args)
                let args = self.call_args(a)?;
                if !rest.is_empty() {
                    return self.unsupported(*l, "a select of a call");
                }
                return Ok(ExprKind::Call(parts.join("."), args).into());
            }
            return self.selects(ExprKind::Hier(parts).into(), &ops[k..]);
        }
        self.selects(ExprKind::Ident(name).into(), &ops[k..])
    }
}

/// `T'(e)` of an assignment pattern: the pattern gets type `T` (also in a conditional's
/// branches).
fn typed_patterns(e: Expr, t: &Rc<super::SvType>) -> Expr {
    let kind = match e.kind {
        ExprKind::Pattern(None, items) => ExprKind::Pattern(Some(t.clone()), items),
        ExprKind::Cond(c, a, b) => ExprKind::Cond(c, Box::new(typed_patterns(*a, t)), Box::new(typed_patterns(*b, t))),
        k => k,
    };
    Expr::new(kind, e.line)
}
