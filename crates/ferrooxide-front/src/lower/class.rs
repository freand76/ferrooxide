//! Classes: properties, methods, `this` and `super`.
//!
//! Inside a method, a property or method of the class (or of a base class) named without an
//! object is reached through `this`: after lowering a method body, such names become
//! `this.p` / `this.m(...)` unless a local declaration (an argument, a variable, a block
//! variable) hides them.

use super::{Lower, SvType};
use crate::ast::*;
use crate::syntax::kw::Kw;
use crate::syntax::tree as t;
use crate::Result;
use std::collections::HashSet;
use std::rc::Rc;

/// The names a class's methods reach through `this` (with those of its base classes).
#[derive(Debug, Default)]
pub(crate) struct ClassNames {
    pub props: HashSet<String>,
    pub methods: HashSet<String>,
}

impl Lower {
    /// `class name [extends base]; items endclass`
    pub(crate) fn class_decl(&mut self, c: &t::ClassDecl) -> Result<ClassDecl> {
        let line = c.loc.line;
        if c.interface {
            return self.unsupported(c.loc, "an interface class");
        }
        if c.params.is_some() {
            return self.unsupported(c.loc, "a parameterized class");
        }
        if !c.implements.is_empty() {
            return self.unsupported(c.loc, "implements");
        }
        let name = self.name(&c.name, c.loc);
        let mut names = ClassNames::default();
        let (base, base_args) = match &c.extends {
            None => (None, None),
            Some((n, args)) => {
                if n.params.is_some() || !n.scopes.is_empty() {
                    return self.unsupported(c.loc, "this base class");
                }
                let Some(b) = self.classes.get(&n.name) else {
                    return self.err(c.loc, format!("unknown base class '{}'", n.name));
                };
                names.props.extend(b.props.iter().cloned());
                names.methods.extend(b.methods.iter().cloned());
                let args = match args {
                    Some(a) => Some(self.call_args(a, c.loc)?),
                    None => None,
                };
                (Some(n.name.clone()), args)
            }
        };
        // the class is a type from here on (also inside itself)
        let mut ct = SvType::vector(NetKind::Class, false, vec![(super::types::num(63), super::types::num(0))]);
        ct.int_kind = true;
        ct.class = Some(name.clone());
        self.types.insert(name.clone(), Rc::new(ct));
        // properties are names in the methods (their types for member chains)
        let saved_vars = self.vars.clone();
        let mut props = Vec::new();
        for it in &c.items {
            let t::ClassItem::Property { quals, decl } = it else { continue };
            if quals.contains(&Kw::Static) {
                return self.unsupported(super::item_loc(decl), "a static class property");
            }
            let t::Item::Var(v) = decl else {
                return self.unsupported(super::item_loc(decl), &format!("{} in a class", super::item_what(decl)));
            };
            if v.lifetime.is_some() {
                return self.unsupported(v.loc, "a class property lifetime");
            }
            let mut d = self.decl_head(None, None, v.var, &v.ty, None, None, v.loc)?;
            if !self.pending.is_empty() {
                return self.unsupported(v.loc, "an enum type declared in a class property");
            }
            d.names = self.declarators(&v.decls)?;
            if d.names.iter().any(|n| n.heap.is_some() || !n.dims.is_empty()) {
                return self.unsupported(v.loc, "an array as a class property");
            }
            for n in &d.names {
                names.props.insert(n.name.clone());
            }
            props.push(d);
        }
        let mut virtuals = Vec::new();
        for it in &c.items {
            if let t::ClassItem::Method(m) = it {
                names.methods.insert(m.name.name.clone());
                if m.quals.contains(&Kw::Virtual) {
                    virtuals.push(m.name.name.clone());
                }
            }
        }
        // (the class's names are known before its methods, which may call each other)
        let names = Rc::new(names);
        self.classes.insert(name.clone(), names.clone());
        for d in &mut props {
            for n in &mut d.names {
                if let Some(e) = &mut n.init {
                    this_expr(e, &names, &[]);
                }
            }
        }
        let mut methods = Vec::new();
        for it in &c.items {
            match it {
                t::ClassItem::Property { .. } => {}
                t::ClassItem::Method(m) => {
                    if let Some(q) = m.quals.iter().find(|q| !matches!(q, Kw::Virtual | Kw::Local | Kw::Protected)) {
                        return self.unsupported(m.loc, &format!("a {} method", q.as_str()));
                    }
                    let mut f = self.subroutine_in(m, true)?;
                    // methods are automatic (IEEE 1800-2017 8.6)
                    if m.lifetime != Some(t::Lifetime::Static) {
                        f.automatic = true;
                    }
                    let mut locals: Vec<String> = vec![f.name.clone()];
                    for d in &f.decls {
                        locals.extend(d.names.iter().map(|n| n.name.clone()));
                    }
                    for d in &mut f.decls {
                        for n in &mut d.names {
                            if let Some(e) = &mut n.init {
                                this_expr(e, &names, &locals);
                            }
                        }
                    }
                    this_stmt(&mut f.body, &names, &mut locals);
                    methods.push(f);
                }
                t::ClassItem::Constraint(k) => return self.unsupported(k.loc, "a constraint"),
                t::ClassItem::Item(t::Item::Empty(_)) => {}
                t::ClassItem::Item(i) => return self.unsupported(super::item_loc(i), &format!("{} in a class", super::item_what(i))),
            }
        }
        self.vars = saved_vars;
        Ok(ClassDecl { name, base, base_args, props, methods, virtuals, abstract_: c.virtual_, line })
    }

    /// The type of a variable named `n` is a class.
    pub(crate) fn is_class_var(&self, n: &str) -> bool {
        n == "this" || n == "super" || self.vars.get(n).is_some_and(|v| v.0.class.is_some() && v.1.is_empty())
    }
}

/// A name of the class reached through `this`, unless a local hides it.
fn this_name(n: &str, c: &ClassNames, locals: &[String]) -> bool {
    c.props.contains(n) && !locals.iter().any(|l| l == n)
}

fn this() -> Box<Expr> {
    Box::new(Expr::Ident("this".into()))
}

/// `this.` before the class's names in an expression.
fn this_expr(e: &mut Expr, c: &ClassNames, locals: &[String]) {
    match e {
        Expr::Ident(n) if this_name(n, c, locals) => *e = Expr::Member(this(), n.clone()),
        Expr::Hier(p) if this_name(&p[0], c, locals) => {
            let mut x = Expr::Member(this(), p[0].clone());
            for m in &p[1..] {
                x = Expr::Member(Box::new(x), m.clone());
            }
            *e = x;
        }
        Expr::Call(n, args) => {
            args.iter_mut().for_each(|a| this_expr(a, c, locals));
            if let Some(x) = this_call(n, c, locals) {
                *e = Expr::MethodCall(Box::new(x.0), x.1, std::mem::take(args));
            }
        }
        Expr::Index(a, b) => {
            this_expr(a, c, locals);
            this_expr(b, c, locals);
        }
        Expr::Range(a, b, d) | Expr::IndexedRange(a, b, d, _) | Expr::Cond(a, b, d) => {
            this_expr(a, c, locals);
            this_expr(b, c, locals);
            this_expr(d, c, locals);
        }
        Expr::Unary(_, a) | Expr::Member(a, _) | Expr::NewCopy(a) | Expr::IncDec { e: a, .. } => this_expr(a, c, locals),
        Expr::Binary(_, a, b) => {
            this_expr(a, c, locals);
            this_expr(b, c, locals);
        }
        Expr::Concat(v) | Expr::SysCall(_, v) | Expr::New(v) => v.iter_mut().for_each(|a| this_expr(a, c, locals)),
        Expr::Repl(n, v) => {
            this_expr(n, c, locals);
            v.iter_mut().for_each(|a| this_expr(a, c, locals));
        }
        Expr::MethodCall(o, _, args) => {
            this_expr(o, c, locals);
            args.iter_mut().for_each(|a| this_expr(a, c, locals));
        }
        Expr::NewArray(n, init) => {
            this_expr(n, c, locals);
            if let Some(i) = init {
                this_expr(i, c, locals);
            }
        }
        Expr::Pattern(_, items) => {
            for it in items {
                match it {
                    PatItem::Pos(x) | PatItem::Key(_, x) | PatItem::Default(x) => this_expr(x, c, locals),
                }
            }
        }
        _ => {}
    }
}

/// A call `m(...)` / `p.m(...)` of a method through `this`: (object, method).
fn this_call(n: &str, c: &ClassNames, locals: &[String]) -> Option<(Expr, String)> {
    match n.rsplit_once('.') {
        None if c.methods.contains(n) && !locals.iter().any(|l| l == n) => Some((Expr::Ident("this".into()), n.to_string())),
        Some((obj, m)) => {
            let parts: Vec<&str> = obj.split('.').collect();
            if !this_name(parts[0], c, locals) {
                return None;
            }
            let mut x = Expr::Member(this(), parts[0].to_string());
            for p in &parts[1..] {
                x = Expr::Member(Box::new(x), p.to_string());
            }
            Some((x, m.to_string()))
        }
        None => None,
    }
}

/// `this.` before the class's names in a statement; `locals` grows with block declarations.
fn this_stmt(s: &mut Stmt, c: &ClassNames, locals: &mut Vec<String>) {
    let ex = |e: &mut Expr, l: &[String]| this_expr(e, c, l);
    match s {
        Stmt::Block { decls, stmts, .. } => {
            let n = locals.len();
            for d in decls.iter_mut() {
                for dl in &mut d.names {
                    if let Some(e) = &mut dl.init {
                        this_expr(e, c, locals);
                    }
                    locals.push(dl.name.clone());
                }
            }
            stmts.iter_mut().for_each(|x| this_stmt(x, c, locals));
            locals.truncate(n);
        }
        Stmt::Assign { lhs, rhs, timing, .. } => {
            ex(lhs, locals);
            ex(rhs, locals);
            if let Some(t) = timing {
                this_timing(t, c, locals);
            }
        }
        Stmt::If { cond, then_s, else_s, .. } => {
            ex(cond, locals);
            this_stmt(then_s, c, locals);
            if let Some(e) = else_s {
                this_stmt(e, c, locals);
            }
        }
        Stmt::Case { expr, arms, default, .. } => {
            ex(expr, locals);
            for (labels, body) in arms {
                labels.iter_mut().for_each(|l| ex(l, locals));
                this_stmt(body, c, locals);
            }
            if let Some(d) = default {
                this_stmt(d, c, locals);
            }
        }
        Stmt::For { init, cond, step, body, .. } => {
            this_stmt(init, c, locals);
            ex(cond, locals);
            this_stmt(step, c, locals);
            this_stmt(body, c, locals);
        }
        Stmt::While { cond, body, .. } | Stmt::DoWhile { cond, body, .. } | Stmt::Wait { cond, body, .. } => {
            ex(cond, locals);
            this_stmt(body, c, locals);
        }
        Stmt::Repeat { count, body, .. } => {
            ex(count, locals);
            this_stmt(body, c, locals);
        }
        Stmt::Forever { body, .. } => this_stmt(body, c, locals),
        Stmt::Timed { timing, body, .. } => {
            this_timing(timing, c, locals);
            this_stmt(body, c, locals);
        }
        Stmt::SysTask { args, .. } => args.iter_mut().flatten().for_each(|a| ex(a, locals)),
        Stmt::TaskCall { name, args, line } => {
            args.iter_mut().for_each(|a| ex(a, locals));
            if let Some((obj, m)) = this_call(name, c, locals) {
                *s = Stmt::MethodCall { obj, name: m, args: std::mem::take(args), line: *line };
            }
        }
        Stmt::MethodCall { obj, args, .. } => {
            ex(obj, locals);
            args.iter_mut().for_each(|a| ex(a, locals));
        }
        Stmt::Force { lhs, rhs, .. } => {
            ex(lhs, locals);
            if let Some(r) = rhs {
                ex(r, locals);
            }
        }
        Stmt::Return { value: Some(v), .. } => ex(v, locals),
        _ => {}
    }
}

fn this_timing(t: &mut Timing, c: &ClassNames, locals: &[String]) {
    let ctl = |ec: &mut EventCtl| {
        if let EventCtl::List(v) = ec {
            v.iter_mut().for_each(|(_, e)| this_expr(e, c, locals));
        }
    };
    match t {
        Timing::Delay(e) => this_expr(e, c, locals),
        Timing::Event(ec) => ctl(ec),
        Timing::Repeat(e, ec) => {
            this_expr(e, c, locals);
            ctl(ec);
        }
    }
}
