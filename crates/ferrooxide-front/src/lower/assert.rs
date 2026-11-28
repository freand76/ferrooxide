//! Concurrent assertions (IEEE 1800-2017 16): the properties and sequences of the syntax tree
//! as those of the AST. Named sequences and properties are expanded where they are used,
//! their formal arguments standing for the actual ones (lowered where the instance is); the
//! clock and `disable iff` come from the property or from the module's default clocking and
//! `default disable iff`.

use super::Lower;
use crate::ast::*;
use crate::syntax::tree as t;
use crate::Result;
use std::collections::HashMap;
use std::rc::Rc;

/// A named sequence or property.
pub(crate) struct SvaDecl {
    ports: Vec<t::TfPort>,
    body: t::Prop,
    property: bool,
    loc: t::Loc,
}

/// The assertion context of a module.
#[derive(Default)]
pub(crate) struct Sva {
    decls: HashMap<String, Rc<SvaDecl>>,
    /// clocking blocks by name, for `default clocking name;`
    clockings: HashMap<String, t::EventCtl>,
    default_clock: Option<t::EventCtl>,
    /// the event of the module's global clocking block (IEEE 1800-2017 14.14)
    pub(crate) global_clock: Option<t::EventCtl>,
    /// a global clocking sampled value function was lowered (`$past_gclk`, ...)
    pub(crate) gclk_used: bool,
    default_disable: Option<t::Expr>,
    /// the named sequences and properties being expanded, innermost last: their formal
    /// arguments and the actual ones
    pub(crate) env: Vec<HashMap<String, t::Expr>>,
    /// `let` declarations
    pub(crate) lets: HashMap<String, Rc<t::LetDecl>>,
}

/// A concurrent assertion's clock and `disable iff`, as found while lowering its property.
#[derive(Default)]
struct Head {
    clock: Option<EventCtl>,
    disable: Option<Expr>,
}

fn boxed<X>(x: X) -> Box<X> {
    Box::new(x)
}

impl Lower {
    /// The module items that set the assertion context (named sequences and properties,
    /// clocking blocks, `default disable iff`), wherever they are in the module.
    pub(crate) fn sva_prescan(&mut self, items: &[&t::Item]) -> Result<()> {
        for it in items {
            self.sva_item(it)?;
        }
        Ok(())
    }

    /// An item of the assertion context; true if `it` is one.
    pub(crate) fn sva_item(&mut self, it: &t::Item) -> Result<bool> {
        match it {
            t::Item::Property(d) => {
                let decl = SvaDecl { ports: d.ports.clone(), body: d.prop.clone(), property: true, loc: d.loc };
                self.sva_decl(&d.name, &d.decls, decl)?;
            }
            t::Item::Sequence(d) => {
                let decl = SvaDecl { ports: d.ports.clone(), body: d.seq.clone(), property: false, loc: d.loc };
                self.sva_decl(&d.name, &d.decls, decl)?;
            }
            t::Item::Clocking(c) => {
                if c.items.iter().any(|i| !matches!(i, t::ClockingItem::Default { .. })) {
                    return self.unsupported(c.loc, "a clocking block with clocking signals or items");
                }
                match c.kind {
                    Some(crate::syntax::kw::Kw::Global) => self.sva.global_clock = Some(c.event.clone()),
                    Some(_) => self.sva.default_clock = Some(c.event.clone()),
                    None => {}
                }
                if let Some(n) = &c.name {
                    self.sva.clockings.insert(n.clone(), c.event.clone());
                }
            }
            t::Item::ClockingRef { global: true, loc, .. } => return self.unsupported(*loc, "a global clocking block"),
            t::Item::ClockingRef { global: false, name, loc } => match self.sva.clockings.get(name) {
                Some(ev) => self.sva.default_clock = Some(ev.clone()),
                None => return self.err(*loc, format!("unknown clocking block '{name}'")),
            },
            t::Item::DefaultDisable(e, _) => self.sva.default_disable = Some(e.clone()),
            t::Item::Let(d) => {
                self.sva.lets.insert(d.name.clone(), Rc::new((**d).clone()));
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn sva_decl(&mut self, name: &str, decls: &[t::Item], decl: SvaDecl) -> Result<()> {
        if !decls.is_empty() {
            return self.unsupported(decl.loc, "a local variable of a sequence or property");
        }
        self.sva.decls.insert(name.to_string(), Rc::new(decl));
        Ok(())
    }

    /// A concurrent assertion (None: one that simulation ignores, `restrict`).
    pub(crate) fn conc_assert(&mut self, a: &t::Assertion) -> Result<Option<ConcAssert>> {
        let (kind, body) = match (&a.kind, &a.body) {
            (t::AssertKind::Restrict, _) => return Ok(None),
            (t::AssertKind::Cover, t::AssertBody::Sequence(p)) => (AssertKind::CoverSeq, p),
            (_, t::AssertBody::Sequence(_)) => return self.err(a.loc, "only `cover` takes a sequence"),
            (t::AssertKind::Assert, t::AssertBody::Property(p)) => (AssertKind::Assert, p),
            (t::AssertKind::Assume, t::AssertBody::Property(p)) => (AssertKind::Assume, p),
            (t::AssertKind::Cover, t::AssertBody::Property(p)) => (AssertKind::Cover, p),
            (_, t::AssertBody::Immediate { .. }) => unreachable!("a deferred assertion"),
        };
        let mut head = Head::default();
        self.sva.gclk_used = false;
        let prop = self.sva_top(body, &mut head, a.loc)?;
        let gclk = std::mem::take(&mut self.sva.gclk_used);
        if kind == AssertKind::CoverSeq && !matches!(prop, Property::Seq(_)) {
            return self.err(a.loc, "`cover sequence` takes a sequence");
        }
        let clock = match head.clock {
            Some(c) => c,
            None => match self.sva.default_clock.clone() {
                Some(ev) => self.event_ctl(&ev, a.loc)?,
                None => return self.unsupported(a.loc, "a concurrent assertion without a clock"),
            },
        };
        // the global clocking sampled value functions: of the assertion's clock only (the
        // registers of their past values shift at its ticks)
        if gclk {
            let g = self.sva.global_clock.clone().unwrap();
            let g = self.event_ctl(&g, a.loc)?;
            if crate::dump::event_ctl(&g) != crate::dump::event_ctl(&clock) {
                return self.unsupported(a.loc, "a global clocking sampled value function in an assertion whose clock is not the global clock");
            }
        }
        let disable = match head.disable {
            Some(d) => Some(d),
            None => match self.sva.default_disable.clone() {
                Some(e) => Some(self.expr(&e)?),
                None => None,
            },
        };
        let line = a.loc;
        let pass = match &a.action.pass {
            Some(s) => self.stmt(s)?,
            None => Stmt::Null,
        };
        let fail = match &a.action.fail {
            Some(s) => self.stmt(s)?,
            // (16.14.1: `$error` when there is no fail statement; `cover` has none)
            None if matches!(kind, AssertKind::Cover | AssertKind::CoverSeq) => Stmt::Null,
            None => Stmt::SysTask { name: "$error".into(), args: Vec::new(), line },
        };
        let label = a.label.as_ref().map(|l| self.name(l, a.loc));
        Ok(Some(ConcAssert { kind, label, clock, disable, prop, pass, fail, line }))
    }

    /// The top of an assertion's property: its clock and `disable iff` go to `head`.
    fn sva_top(&mut self, p: &t::Prop, head: &mut Head, loc: t::Loc) -> Result<Property> {
        match p {
            t::Prop::Clocked(ev, inner) => {
                if head.clock.is_some() {
                    return self.unsupported(loc, "a property with several clocks");
                }
                head.clock = Some(self.event_ctl(ev, loc)?);
                self.sva_top(inner, head, loc)
            }
            t::Prop::DisableIff(e, inner) => {
                if head.disable.is_some() {
                    return self.err(loc, "a property with two `disable iff`");
                }
                head.disable = Some(self.expr(e)?);
                self.sva_top(inner, head, loc)
            }
            t::Prop::Expr(e) => match self.sva_ref(e)? {
                Some((d, env)) => self.sva_expand(&d, env, |l, body| l.sva_top(body, head, loc)),
                None => self.sva_prop(p),
            },
            _ => self.sva_prop(p),
        }
    }

    /// A use of a named sequence or property (`s`, `s(args)`): its declaration and the
    /// actual arguments of its formal ones.
    fn sva_ref(&mut self, e: &t::Expr) -> Result<Option<(Rc<SvaDecl>, HashMap<String, t::Expr>)>> {
        let (name, args): (&str, &[t::Arg]) = match &e.kind {
            t::ExprKind::Ident(n) => (n, &[]),
            t::ExprKind::Call { func, args, with_: None, .. } => match &func.kind {
                t::ExprKind::Ident(n) => (n, args),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        // (a formal argument hides a named sequence of its name)
        if self.sva.env.last().is_some_and(|m| m.contains_key(name)) {
            return Ok(None);
        }
        let Some(d) = self.sva.decls.get(name).cloned() else { return Ok(None) };
        let env = self.bind_formals(&d.ports, args, name, e.loc)?;
        Ok(Some((d, env)))
    }

    /// The actual arguments of formal arguments `ports` of `name` (a named sequence or
    /// property, a `let`) from the arguments of a use: positional, named, defaults.
    fn bind_formals(&mut self, ports: &[t::TfPort], args: &[t::Arg], name: &str, loc: t::Loc) -> Result<HashMap<String, t::Expr>> {
        let mut env = HashMap::new();
        for (k, a) in args.iter().enumerate() {
            let (port, actual) = match a {
                t::Arg::Pos(x) => match ports.get(k) {
                    Some(p) => (p, x),
                    None => return self.err(loc, format!("too many arguments to '{name}'")),
                },
                t::Arg::Named(n, x) => match ports.iter().find(|p| &p.name == n) {
                    Some(p) => (p, x),
                    None => return self.err(loc, format!("'{name}' has no argument '{n}'")),
                },
            };
            if let Some(x) = actual {
                if env.insert(port.name.clone(), x.clone()).is_some() {
                    return self.err(loc, format!("argument '{}' of '{name}' is given twice", port.name));
                }
            }
        }
        for p in ports {
            if !env.contains_key(&p.name) {
                match &p.default {
                    Some(x) => {
                        env.insert(p.name.clone(), x.clone());
                    }
                    None => return self.err(loc, format!("argument '{}' of '{name}' has no value", p.name)),
                }
            }
        }
        Ok(env)
    }

    /// A use of a `let` declaration (IEEE 1800-2017 11.12), `name` or `name(args)`: its
    /// expression with the formal arguments standing for the actual ones (cast to the type of
    /// a typed formal); None if `e` is none.
    pub(crate) fn let_use(&mut self, e: &t::Expr) -> Result<Option<Expr>> {
        let (name, args): (&str, &[t::Arg]) = match &e.kind {
            t::ExprKind::Ident(n) => (n, &[]),
            t::ExprKind::Call { func, args, with_: None, parens: true } => match &func.kind {
                t::ExprKind::Ident(n) => (n, args),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        // (a formal argument hides a let of its name)
        if self.sva.env.last().is_some_and(|m| m.contains_key(name)) {
            return Ok(None);
        }
        let Some(d) = self.sva.lets.get(name).cloned() else { return Ok(None) };
        let mut env = self.bind_formals(&d.ports, args, name, e.loc)?;
        // (a port without a type takes the previous one's, unless it has a direction)
        let mut ty: Option<&t::DataType> = None;
        for p in &d.ports {
            if p.dir.is_some() {
                return self.err(p.loc, "a let argument cannot have a direction");
            }
            if !p.dims.is_empty() {
                return self.unsupported(p.loc, "an unpacked dimension of a let argument");
            }
            if let Some(t) = &p.ty {
                ty = Some(t);
            }
            // (`untyped`: no cast)
            let Some(t) = ty.filter(|t| !(matches!(t.kind, t::TypeKind::Implicit) && t.packed.is_empty() && t.signed.is_none())) else { continue };
            let a = env.get_mut(&p.name).unwrap();
            let loc = a.loc;
            *a = t::Expr::new(t::ExprKind::Cast(t::CastTarget::Type(Box::new(t.clone())), Box::new(a.clone())), loc);
        }
        if self.sva.env.len() > 64 {
            return self.err(d.loc, "let declarations nested too deeply (a recursive one?)");
        }
        self.sva.env.push(env);
        let r = self.expr(&d.expr);
        self.sva.env.pop();
        r.map(Some)
    }

    /// `f` on the body of named sequence or property `d` with its formal arguments bound.
    fn sva_expand<X>(&mut self, d: &SvaDecl, env: HashMap<String, t::Expr>, f: impl FnOnce(&mut Self, &t::Prop) -> Result<X>) -> Result<X> {
        if self.sva.env.len() > 64 {
            return self.err(d.loc, "named sequences or properties nested too deeply (a recursive one?)");
        }
        self.sva.env.push(env);
        let r = f(self, &d.body);
        self.sva.env.pop();
        r
    }

    /// `f` where the actual arguments of the innermost expansion were written.
    pub(crate) fn sva_caller<X>(&mut self, f: impl FnOnce(&mut Self) -> Result<X>) -> Result<X> {
        let top = self.sva.env.pop();
        let r = f(self);
        if let Some(t) = top {
            self.sva.env.push(t);
        }
        r
    }

    /// The actual argument for formal argument `n` of the innermost expansion.
    pub(crate) fn sva_actual(&self, n: &str) -> Option<t::Expr> {
        self.sva.env.last()?.get(n).cloned()
    }

    /// Whether `p` is a sequence (else a property).
    fn sva_is_seq(&mut self, p: &t::Prop) -> bool {
        match p {
            t::Prop::Expr(e) => {
                if let t::ExprKind::Ident(n) = &e.kind {
                    if let Some(a) = self.sva_actual(n) {
                        return match &a.kind {
                            t::ExprKind::Prop(q) => {
                                let q = (**q).clone();
                                self.sva_caller(|l| Ok(l.sva_is_seq(&q))).unwrap_or(true)
                            }
                            _ => true,
                        };
                    }
                }
                match self.sva_ref(e) {
                    Ok(Some((d, env))) => {
                        let d2 = d.clone();
                        !d.property || self.sva_expand(&d2, env, |l, b| Ok(l.sva_is_seq(b))).unwrap_or(false)
                    }
                    _ => true,
                }
            }
            t::Prop::Delay(..) | t::Prop::Repeat(..) | t::Prop::FirstMatch(..) | t::Prop::MatchItems(..) => true,
            t::Prop::Binary(t::PropOp::Or | t::PropOp::And | t::PropOp::Intersect | t::PropOp::Within | t::PropOp::Throughout, a, b) => {
                self.sva_is_seq(a) && self.sva_is_seq(b)
            }
            _ => false,
        }
    }

    /// A property.
    fn sva_prop(&mut self, p: &t::Prop) -> Result<Property> {
        if self.sva_is_seq(p) {
            return Ok(Property::Seq(self.sva_seq(p)?));
        }
        let loc = prop_loc(p);
        Ok(match p {
            t::Prop::Expr(e) => {
                if let t::ExprKind::Ident(n) = &e.kind {
                    if let Some(a) = self.sva_actual(n) {
                        return self.sva_caller(|l| match &a.kind {
                            t::ExprKind::Prop(q) => l.sva_prop(q),
                            _ => Ok(Property::Seq(Sequence::Bool(l.expr(&a)?))),
                        });
                    }
                }
                if let t::ExprKind::Prop(q) = &e.kind {
                    return self.sva_prop(q);
                }
                match self.sva_ref(e)? {
                    Some((d, env)) => self.sva_expand(&d, env, |l, body| match body {
                        t::Prop::Clocked(..) | t::Prop::DisableIff(..) => l.unsupported(loc, "a clock or `disable iff` inside a property"),
                        _ => l.sva_prop(body),
                    })?,
                    None => Property::Seq(Sequence::Bool(self.expr(e)?)),
                }
            }
            t::Prop::Not(a) => Property::Not(boxed(self.sva_prop(a)?)),
            t::Prop::Binary(op, a, b) => match op {
                t::PropOp::ImplOverlap | t::PropOp::ImplNext => {
                    if !self.sva_is_seq(a) {
                        return self.err(loc, "the left side of an implication must be a sequence");
                    }
                    Property::Implies { ante: self.sva_seq(a)?, next: *op == t::PropOp::ImplNext, cons: boxed(self.sva_prop(b)?) }
                }
                t::PropOp::And => Property::And(boxed(self.sva_prop(a)?), boxed(self.sva_prop(b)?)),
                t::PropOp::Or => Property::Or(boxed(self.sva_prop(a)?), boxed(self.sva_prop(b)?)),
                // `a implies b`: `not a or b`; `a iff b`: both or neither
                t::PropOp::Implies => Property::Or(boxed(Property::Not(boxed(self.sva_prop(a)?))), boxed(self.sva_prop(b)?)),
                t::PropOp::Iff => {
                    let (x, y) = (self.sva_prop(a)?, self.sva_prop(b)?);
                    let both = Property::And(boxed(x.clone()), boxed(y.clone()));
                    let neither = Property::And(boxed(Property::Not(boxed(x))), boxed(Property::Not(boxed(y))));
                    Property::Or(boxed(both), boxed(neither))
                }
                t::PropOp::FollowOverlap | t::PropOp::FollowNext => return self.unsupported(loc, "a followed-by property (`#-#`, `#=#`)"),
                t::PropOp::Until | t::PropOp::SUntil | t::PropOp::UntilWith | t::PropOp::SUntilWith => {
                    return self.unsupported(loc, "an `until` property")
                }
                t::PropOp::Intersect | t::PropOp::Within | t::PropOp::Throughout => {
                    return self.err(loc, "this operator takes sequences, not properties")
                }
            },
            t::Prop::If(c, a, b) => {
                let c = self.expr(c)?;
                let b = match b {
                    Some(b) => Some(boxed(self.sva_prop(b)?)),
                    None => None,
                };
                Property::If(c, boxed(self.sva_prop(a)?), b)
            }
            t::Prop::Strength(false, a) => self.sva_prop(a)?,
            t::Prop::Strength(true, _) => return self.unsupported(loc, "a strong property"),
            t::Prop::Unary(..) => return self.unsupported(loc, "this property operator (`nexttime`, `always`, `eventually`)"),
            t::Prop::Case(..) => return self.unsupported(loc, "a `case` property"),
            t::Prop::Abort(..) => return self.unsupported(loc, "an abort property (`accept_on`, `reject_on`)"),
            t::Prop::Clocked(..) => return self.unsupported(loc, "a clock inside a property"),
            t::Prop::DisableIff(..) => return self.unsupported(loc, "`disable iff` inside a property"),
            t::Prop::Delay(..) | t::Prop::Repeat(..) | t::Prop::FirstMatch(..) | t::Prop::MatchItems(..) => unreachable!("sequences"),
        })
    }

    /// A sequence.
    fn sva_seq(&mut self, p: &t::Prop) -> Result<Sequence> {
        let loc = prop_loc(p);
        let bs = |l: &mut Self, x: &t::Prop| -> Result<Box<Sequence>> { Ok(boxed(l.sva_seq(x)?)) };
        Ok(match p {
            t::Prop::Expr(e) => {
                if let t::ExprKind::Ident(n) = &e.kind {
                    if let Some(a) = self.sva_actual(n) {
                        return self.sva_caller(|l| match &a.kind {
                            t::ExprKind::Prop(q) => l.sva_seq(q),
                            _ => Ok(Sequence::Bool(l.expr(&a)?)),
                        });
                    }
                }
                if let t::ExprKind::Prop(q) = &e.kind {
                    return self.sva_seq(q);
                }
                match self.sva_ref(e)? {
                    Some((d, _)) if d.property => return self.err(loc, "a property where a sequence is expected"),
                    Some((d, env)) => self.sva_expand(&d, env, |l, body| match body {
                        t::Prop::Clocked(..) => l.unsupported(loc, "a clocked sequence inside another sequence or property"),
                        _ => l.sva_seq(body),
                    })?,
                    None => Sequence::Bool(self.expr(e)?),
                }
            }
            t::Prop::Delay(a, r, b) => {
                let a = match a {
                    Some(a) => Some(bs(self, a)?),
                    None => None,
                };
                Sequence::Delay(a, self.cycles(r)?, bs(self, b)?)
            }
            t::Prop::Repeat(s, kind, r) => {
                let c = match (kind, r) {
                    (t::RepeatKind::Plus, _) => Cycles { lo: num(1), hi: None, unbounded: true },
                    (t::RepeatKind::Consecutive, None) => Cycles { lo: num(0), hi: None, unbounded: true },
                    (_, Some(r)) => self.cycles(r)?,
                    (_, None) => return self.err(loc, "this repetition needs a count"),
                };
                match kind {
                    t::RepeatKind::Consecutive | t::RepeatKind::Plus => Sequence::Repeat(bs(self, s)?, c),
                    t::RepeatKind::Goto | t::RepeatKind::NonConsecutive => {
                        let Sequence::Bool(b) = self.sva_seq(s)? else {
                            return self.err(loc, "a goto or nonconsecutive repetition needs a boolean expression");
                        };
                        if *kind == t::RepeatKind::Goto {
                            Sequence::Goto(b, c)
                        } else {
                            Sequence::NonCons(b, c)
                        }
                    }
                }
            }
            t::Prop::Binary(op, a, b) => match op {
                t::PropOp::Or => Sequence::Or(bs(self, a)?, bs(self, b)?),
                t::PropOp::And => Sequence::And(bs(self, a)?, bs(self, b)?),
                t::PropOp::Intersect => Sequence::Intersect(bs(self, a)?, bs(self, b)?),
                t::PropOp::Within => Sequence::Within(bs(self, a)?, bs(self, b)?),
                t::PropOp::Throughout => {
                    let Sequence::Bool(c) = self.sva_seq(a)? else {
                        return self.err(loc, "the left side of `throughout` must be a boolean expression");
                    };
                    Sequence::Throughout(c, bs(self, b)?)
                }
                _ => return self.err(loc, "a property where a sequence is expected"),
            },
            t::Prop::FirstMatch(..) => return self.unsupported(loc, "first_match"),
            t::Prop::MatchItems(..) => return self.unsupported(loc, "a sequence with match items (local variables)"),
            t::Prop::Clocked(..) => return self.unsupported(loc, "a clock inside a sequence"),
            _ => return self.err(loc, "a property where a sequence is expected"),
        })
    }

    fn cycles(&mut self, r: &t::CycleRange) -> Result<Cycles> {
        Ok(match r {
            t::CycleRange::One(e) => Cycles { lo: self.expr(e)?, hi: None, unbounded: false },
            t::CycleRange::Range(a, b) if matches!(b.kind, t::ExprKind::Dollar) => Cycles { lo: self.expr(a)?, hi: None, unbounded: true },
            t::CycleRange::Range(a, b) => Cycles { lo: self.expr(a)?, hi: Some(self.expr(b)?), unbounded: false },
        })
    }
}

fn num(v: u64) -> Expr {
    ExprKind::Num(dec(v)).into()
}

/// A location of a property for messages: its first expression's.
fn prop_loc(p: &t::Prop) -> t::Loc {
    match p {
        t::Prop::Expr(e) | t::Prop::If(e, ..) | t::Prop::Case(e, _) | t::Prop::Abort(_, e, _) | t::Prop::DisableIff(e, _) => e.loc,
        t::Prop::MatchItems(a, _) | t::Prop::Repeat(a, ..) | t::Prop::Binary(_, a, _) | t::Prop::Not(a) | t::Prop::Strength(_, a) | t::Prop::Unary(.., a) | t::Prop::FirstMatch(a, _) | t::Prop::Clocked(_, a) => prop_loc(a),
        t::Prop::Delay(Some(a), ..) => prop_loc(a),
        t::Prop::Delay(None, r, _) => match r {
            t::CycleRange::One(e) | t::CycleRange::Range(e, _) => e.loc,
        },
    }
}
