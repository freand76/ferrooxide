//! Classes: properties, methods, `this` and `super`.
//!
//! Inside a method, a property or method of the class (or of a base class) named without an
//! object is reached through `this`: after lowering a method body, such names become
//! `this.p` / `this.m(...)` unless a local declaration (an argument, a variable, a block
//! variable) hides them.
//!
//! A static property is one variable `C$p` next to the class (in the module, package or
//! `$unit` declaring it); `p` in a method, `obj.p` and `C::p` all name it.

use super::{Lower, SvType};
use crate::ast::*;
use crate::syntax::kw::Kw;
use crate::syntax::tree as t;
use crate::Result;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// The names a class's methods reach through `this` (with those of its base classes).
#[derive(Debug, Default)]
pub(crate) struct ClassNames {
    /// the class
    pub name: String,
    pub props: HashSet<String>,
    pub methods: HashSet<String>,
    /// static methods (also of the base classes): called without `this`
    pub static_methods: HashSet<String>,
    /// static properties: name -> the variable holding it
    pub statics: HashMap<String, String>,
    /// typedefs declared in the class (`C::T`)
    pub types: HashMap<String, Rc<SvType>>,
    /// parameters and enum constants declared in the class (`C::P`, `obj.P`)
    pub consts: HashSet<String>,
}

impl Lower {
    /// A name class `class` declares: an error when it declares it already.
    fn own_name(&self, own: &mut HashSet<String>, n: &str, line: Line, class: &str) -> Result<()> {
        if !own.insert(n.to_string()) {
            return Err(crate::Error { msg: format!("'{n}' is already declared in class '{class}'"), file: self.file.clone(), line });
        }
        Ok(())
    }

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
        let mut names = ClassNames { name: name.clone(), ..Default::default() };
        let (base, base_args) = match &c.extends {
            None => (None, None),
            Some((n, args)) => {
                if n.params.is_some() || n.scopes.len() > 1 || n.scopes.first().is_some_and(|s| s.params.is_some()) {
                    return self.unsupported(c.loc, "this base class");
                }
                // `extends P::B`: a class of package P (named `P::B` for the compiler)
                let (b, bname) = match n.scopes.first() {
                    Some(s) => {
                        let Some(b) = self.packages.get(&s.name).and_then(|p| p.classes.get(&n.name)).cloned() else {
                            return self.err(c.loc, format!("unknown base class '{}::{}'", s.name, n.name));
                        };
                        self.use_package(&s.name);
                        (b, format!("{}::{}", s.name, n.name))
                    }
                    None => {
                        let Some(b) = self.classes.get(&n.name).cloned() else {
                            if self.types.get(&n.name).is_some_and(|t| t.class.is_some()) {
                                return self.unsupported(c.loc, "a base class declared after the class");
                            }
                            return self.err(c.loc, format!("unknown base class '{}'", n.name));
                        };
                        (b, n.name.clone())
                    }
                };
                names.props.extend(b.props.iter().cloned());
                names.methods.extend(b.methods.iter().cloned());
                names.static_methods.extend(b.static_methods.iter().cloned());
                names.statics.extend(b.statics.iter().map(|(k, v)| (k.clone(), v.clone())));
                names.types.extend(b.types.iter().map(|(k, v)| (k.clone(), v.clone())));
                names.consts.extend(b.consts.iter().cloned());
                let args = match args {
                    Some(a) => Some(self.call_args(a)?),
                    None => None,
                };
                (Some(bname), args)
            }
        };
        // the class is a type from here on (also inside itself)
        self.types.insert(name.clone(), Self::class_type(&name));
        // (typedefs of a base class are visible in this one; its own stay inside)
        let saved_types = self.types.clone();
        for (k, v) in &names.types {
            self.types.insert(k.clone(), v.clone());
        }
        // properties are names in the methods (their types for member chains)
        let saved_vars = self.vars.clone();
        let mut props = Vec::new();
        let mut statics = Vec::new();
        let mut consts = Vec::new();
        let mut hidden: Vec<(String, bool)> = Vec::new();
        // the names the class itself declares (one item each, IEEE 1800-2017 8.3)
        let mut own: HashSet<String> = HashSet::new();
        let vis = |quals: &[Kw]| {
            if quals.contains(&Kw::Local) {
                Some(true)
            } else if quals.contains(&Kw::Protected) {
                Some(false)
            } else {
                None
            }
        };
        let items: Vec<&t::Item> = c.items.iter().filter_map(|i| if let t::ClassItem::Item(x) = i { Some(x) } else { None }).collect();
        for it in &c.items {
            // parameters and typedefs (enum constants are parameters) in source order with
            // the properties that may use them
            if let t::ClassItem::Item(i @ (t::Item::Param(_) | t::Item::Typedef(_))) = it {
                let before = self.types.clone();
                let mut v = Vec::new();
                // constants in the class's types refer to the class's parameters (`C::W`)
                let known: HashMap<String, String> = consts.iter().map(|p: &ParamDecl| p.name.clone()).chain(names.consts.iter().cloned()).map(|n| (n, name.clone())).collect();
                let saved_scope = self.type_scope.replace(known);
                let r = self.module_item(i, &items, &mut v);
                self.type_scope = saved_scope;
                r?;
                for x in v.into_iter().chain(std::mem::take(&mut self.pending)) {
                    match x {
                        Item::Param(p) => {
                            self.own_name(&mut own, &p.name, p.line, &name)?;
                            consts.push(p)
                        }
                        _ => return self.unsupported(super::item_loc(i), &format!("{} in a class", super::item_what(i))),
                    }
                }
                if let t::Item::Typedef(t::Typedef::Type { name: n, loc, .. }) = i {
                    self.own_name(&mut own, n, loc.line, &name)?;
                }
                for (k, t) in &self.types {
                    if before.get(k).is_none_or(|o| !Rc::ptr_eq(o, t)) {
                        names.types.insert(k.clone(), t.clone());
                    }
                }
                continue;
            }
            let t::ClassItem::Property { quals, decl } = it else { continue };
            let is_static = quals.contains(&Kw::Static);
            let t::Item::Var(v) = decl else {
                return self.unsupported(super::item_loc(decl), &format!("{} in a class", super::item_what(decl)));
            };
            if v.lifetime.is_some() {
                return self.unsupported(v.loc, "a class property lifetime");
            }
            let mut d = self.decl_head(None, None, v.var, &v.ty, None, None, v.loc)?;
            // an enum type declared here: its constants belong to the class
            for x in std::mem::take(&mut self.pending) {
                match x {
                    Item::Param(p) => {
                        self.own_name(&mut own, &p.name, p.line, &name)?;
                        consts.push(p)
                    }
                    _ => return self.unsupported(v.loc, "this property type"),
                }
            }
            d.names = self.declarators(&v.decls)?;
            for n in &d.names {
                self.own_name(&mut own, &n.name, n.line, &name)?;
            }
            if is_static {
                for n in &mut d.names {
                    let var = format!("{name}${}", n.name);
                    names.statics.insert(n.name.clone(), var.clone());
                    n.name = var;
                }
                statics.push(d);
                continue;
            }
            for n in &d.names {
                names.props.insert(n.name.clone());
                if let Some(l) = vis(quals) {
                    hidden.push((n.name.clone(), l));
                }
            }
            props.push(d);
        }
        let mut virtuals = Vec::new();
        let mut static_methods = Vec::new();
        for it in &c.items {
            if let t::ClassItem::Method(m) = it {
                let n = &m.name.name;
                self.own_name(&mut own, n, m.loc.line, &name)?;
                names.methods.insert(n.clone());
                if let Some(l) = vis(&m.quals) {
                    hidden.push((n.clone(), l));
                }
                // (a method of this class hides a static one of a base class)
                names.static_methods.remove(n);
                if m.quals.contains(&Kw::Virtual) {
                    virtuals.push(n.clone());
                }
                if m.quals.contains(&Kw::Static) {
                    if m.quals.contains(&Kw::Virtual) || n == "new" {
                        return self.err(m.loc, format!("method '{n}' cannot be static"));
                    }
                    names.static_methods.insert(n.clone());
                    static_methods.push(n.clone());
                }
            }
        }
        names.consts.extend(consts.iter().map(|p| p.name.clone()));
        // (the class's names are known before its methods, which may call each other)
        let names = Rc::new(names);
        self.classes.insert(name.clone(), names.clone());
        for d in props.iter_mut().chain(statics.iter_mut()) {
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
                    if let Some(q) = m.quals.iter().find(|q| !matches!(q, Kw::Virtual | Kw::Local | Kw::Protected | Kw::Static)) {
                        return self.unsupported(m.loc, &format!("a {} method", q.as_str()));
                    }
                    // (IEEE 1800-2017 8.6: methods have automatic lifetime)
                    if m.lifetime == Some(t::Lifetime::Static) {
                        return self.err(m.loc, format!("method '{}' cannot have a static lifetime", m.name.name));
                    }
                    let mut f = self.subroutine_in(m, true)?;
                    f.automatic = true;
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
                t::ClassItem::Item(t::Item::Empty(_) | t::Item::Param(_) | t::Item::Typedef(_)) => {}
                t::ClassItem::Item(i) => return self.unsupported(super::item_loc(i), &format!("{} in a class", super::item_what(i))),
            }
        }
        // the static properties' variables, with their types, for the declaring scope
        let types: Vec<(String, Option<(Rc<SvType>, Vec<Range>)>)> =
            names.statics.iter().filter(|(_, v)| v.starts_with(&format!("{name}$"))).map(|(p, v)| (v.clone(), self.vars.get(p).cloned())).collect();
        self.vars = saved_vars;
        self.types = saved_types;
        for (v, t) in types {
            if let Some(t) = t {
                self.vars.insert(v, t);
            }
        }
        self.class_statics = statics.into_iter().map(Item::Decl).collect();
        Ok(ClassDecl { name, base, base_args, props, consts, methods, virtuals, statics: static_methods, abstract_: c.virtual_, hidden, line })
    }

    /// The type of handles of class `name`.
    pub(crate) fn class_type(name: &str) -> Rc<SvType> {
        let mut ct = SvType::vector(NetKind::Class, false, vec![(super::types::num(63), super::types::num(0))]);
        ct.int_kind = true;
        ct.class = Some(name.to_string());
        Rc::new(ct)
    }

    /// The type of a variable named `n` is a class.
    pub(crate) fn is_class_var(&self, n: &str) -> bool {
        n == "this" || n == "super" || self.vars.get(n).is_some_and(|v| v.0.class.is_some())
    }
}

impl Lower {
    /// The variable of static property `m` of the class of `e` (a class variable or `C`).
    pub(crate) fn static_prop(&self, e: &Expr, m: &str) -> Option<String> {
        let class = match e {
            Expr::Ident(v) => self.vars.get(v).and_then(|t| t.0.class.clone()).or_else(|| self.classes.contains_key(v).then(|| v.clone()))?,
            _ => return None,
        };
        self.classes.get(&class)?.statics.get(m).cloned()
    }
}

/// A static property of the class, unless a local hides it: its variable.
fn static_name<'c>(n: &str, c: &'c ClassNames, locals: &[String]) -> Option<&'c String> {
    c.statics.get(n).filter(|_| !locals.iter().any(|l| l == n))
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
        Expr::Ident(n) if static_name(n, c, locals).is_some() => *e = Expr::Ident(static_name(n, c, locals).unwrap().clone()),
        Expr::Hier(p) if static_name(&p[0], c, locals).is_some() => p[0] = static_name(&p[0], c, locals).unwrap().clone(),
        Expr::Member(o, n) if matches!(&**o, Expr::Ident(t) if t == "this") && c.statics.contains_key(n.as_str()) => {
            *e = Expr::Ident(c.statics[n.as_str()].clone())
        }
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
            if static_call(n, c, locals) {
                *e = Expr::StaticCall(c.name.clone(), n.clone(), std::mem::take(args));
            } else if let Some(x) = this_call(n, c, locals) {
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
        Expr::Concat(v) | Expr::SysCall(_, v) | Expr::New(v) | Expr::NewTyped(_, v) | Expr::StaticCall(_, _, v) => {
            v.iter_mut().for_each(|a| this_expr(a, c, locals))
        }
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
                    PatItem::Index(k, x) => {
                        this_expr(k, c, locals);
                        this_expr(x, c, locals);
                    }
                }
            }
        }
        _ => {}
    }
}

/// A call `m(...)` of a static method of the class.
fn static_call(n: &str, c: &ClassNames, locals: &[String]) -> bool {
    c.static_methods.contains(n) && !locals.iter().any(|l| l == n)
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
            if static_call(name, c, locals) {
                *s = Stmt::StaticCall { class: c.name.clone(), name: name.clone(), args: std::mem::take(args), line: *line };
            } else if let Some((obj, m)) = this_call(name, c, locals) {
                *s = Stmt::MethodCall { obj, name: m, args: std::mem::take(args), line: *line };
            }
        }
        Stmt::MethodCall { obj, args, .. } => {
            ex(obj, locals);
            args.iter_mut().for_each(|a| ex(a, locals));
        }
        Stmt::StaticCall { args, .. } => args.iter_mut().for_each(|a| ex(a, locals)),
        Stmt::Force { lhs, rhs, .. } => {
            ex(lhs, locals);
            if let Some(r) = rhs {
                ex(r, locals);
            }
        }
        Stmt::Return { value: Some(v), .. } | Stmt::Discard { e: v, .. } => ex(v, locals),
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
