//! Constraints and randomization (IEEE 1800-2017 18): constraint blocks of classes, the
//! constraint items of `randomize() with {...}`, and the calls of `randomize`.

use super::Lower;
use crate::ast::*;
use crate::syntax::tree as t;
use crate::Result;

/// The forms of a call of `randomize`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RandForm {
    /// `randomize(...)`: of `this` in a method, else of variables (as `std::randomize`)
    Bare,
    /// `obj.randomize(...)`
    Obj,
    /// `std::randomize(...)`
    Std,
}

impl Lower {
    /// `func` names `randomize` (the function of a call).
    pub(crate) fn randomize_form(func: &t::Expr) -> Option<RandForm> {
        match &func.kind {
            t::ExprKind::Ident(n) if n == "randomize" => Some(RandForm::Bare),
            t::ExprKind::Member(_, m) if m == "randomize" => Some(RandForm::Obj),
            t::ExprKind::Scoped(n) if n.name == "randomize" && n.params.is_none() && n.scopes.len() == 1 && n.scopes[0].name == "std" => Some(RandForm::Std),
            _ => None,
        }
    }

    /// A call of `randomize`: `obj.randomize(vars) with {items}`, `randomize(...)`,
    /// `std::randomize(...)`.
    pub(crate) fn randomize(&mut self, func: &t::Expr, args: &[t::Arg], with_: Option<&t::With>, loc: t::Loc) -> Result<Expr> {
        let form = Self::randomize_form(func).unwrap();
        // (a bare call in a class is of `this`; elsewhere it randomizes variables)
        let std = form == RandForm::Std || (form == RandForm::Bare && self.in_class.is_none());
        let obj = match &func.kind {
            t::ExprKind::Member(o, _) => Some(Box::new(self.expr(o)?)),
            _ => None,
        };
        // names in the arguments and constraints are the object's properties first
        // (IEEE 1800-2017 18.7); `local::x` is the caller's
        let names = match &obj {
            Some(o) => self.class_of_expr(o),
            None => None,
        };
        let saved = std::mem::replace(&mut self.with_obj, names.map(|n| (obj.as_deref().unwrap().clone(), n)));
        let r = (|| -> Result<(Vec<Expr>, Option<Vec<CItem>>)> {
            let mut vars = Vec::new();
            for a in args {
                match a {
                    t::Arg::Pos(Some(e)) => vars.push(self.expr(e)?),
                    // `randomize(null)`: checks the constraints only
                    t::Arg::Pos(None) | t::Arg::Named(..) => return self.unsupported(loc, "these arguments of randomize()"),
                }
            }
            let with = match with_ {
                None => None,
                Some(t::With::Constraint(None, items)) => Some(self.constraint_items(items)?),
                Some(t::With::Constraint(Some(_), _)) => return self.unsupported(loc, "a randomize() with an identifier list"),
                Some(t::With::Expr(_)) => return self.err(loc, "randomize() takes constraints with `with { ... }`"),
            };
            Ok((vars, with))
        })();
        self.with_obj = saved;
        let (vars, with) = r?;
        if vars.iter().any(|v| matches!(v, Expr::Null)) {
            return self.unsupported(loc, "randomize(null)");
        }
        Ok(Expr::Randomize { obj, vars, with, std })
    }

    /// The names of the class of a class handle expression (a variable, `this`).
    fn class_of_expr(&self, e: &Expr) -> Option<std::rc::Rc<super::class::ClassNames>> {
        let c = match e {
            Expr::Ident(n) if n == "this" => self.in_class.clone()?,
            Expr::Ident(n) => self.vars.get(n)?.0.class.clone()?,
            _ => return None,
        };
        self.classes.get(&c).cloned()
    }

    /// Constraint items.
    pub(crate) fn constraint_items(&mut self, v: &[t::ConstraintItem]) -> Result<Vec<CItem>> {
        let mut out = Vec::new();
        for it in v {
            match it {
                t::ConstraintItem::Expr { soft, expr } => out.push(match &expr.kind {
                    t::ExprKind::Inside(a, set) => {
                        let e = self.expr(a)?;
                        let mut s = Vec::new();
                        for r in set {
                            s.push(self.value_range(r)?);
                        }
                        CItem::Inside { soft: *soft, e, set: s }
                    }
                    t::ExprKind::Dist(a, items) => {
                        let e = self.expr(a)?;
                        let mut d = Vec::new();
                        for x in items {
                            let (lo, hi) = self.value_range(&x.range)?;
                            let (per_value, weight) = match &x.weight {
                                None => (true, Expr::Num(dec(1))),
                                Some((per_range, w)) => (!per_range, self.expr(w)?),
                            };
                            d.push(DistItem { lo, hi, per_value, weight });
                        }
                        CItem::Dist { soft: *soft, e, items: d }
                    }
                    _ => CItem::Expr { soft: *soft, e: self.expr(expr)? },
                }),
                t::ConstraintItem::Implies(g, items) => out.push(CItem::Implies(self.expr(g)?, self.constraint_items(items)?)),
                t::ConstraintItem::If(c, a, b) => {
                    let b = match b {
                        Some(b) => self.constraint_items(b)?,
                        None => Vec::new(),
                    };
                    out.push(CItem::If(self.expr(c)?, self.constraint_items(a)?, b));
                }
                t::ConstraintItem::Foreach(arr, vars, items) => {
                    // (the loop variables are names of the items, not of the class)
                    let n = self.wild.locals.len();
                    self.wild.locals.extend(vars.iter().flatten().cloned());
                    let r = (|| Ok::<_, crate::Error>((self.expr(arr)?, self.constraint_items(items)?)))();
                    self.wild.locals.truncate(n);
                    let (arr, items) = r?;
                    out.push(CItem::Foreach { arr, vars: vars.clone(), items });
                }
                t::ConstraintItem::Solve(..) => out.push(CItem::SolveBefore),
                t::ConstraintItem::Unique(rs) => {
                    let mut v = Vec::new();
                    for r in rs {
                        match r {
                            t::ValueRange::Value(e) => v.push(self.expr(e)?),
                            t::ValueRange::Range(e, _) => return self.unsupported(e.loc, "a range in unique"),
                        }
                    }
                    out.push(CItem::Unique(v));
                }
                t::ConstraintItem::DisableSoft(e) => out.push(CItem::DisableSoft(self.expr(e)?)),
                t::ConstraintItem::Block(items) => out.extend(self.constraint_items(items)?),
            }
        }
        Ok(out)
    }

    /// A value or a range `[lo:hi]` of `inside` or `dist`.
    fn value_range(&mut self, r: &t::ValueRange) -> Result<(Expr, Option<Expr>)> {
        Ok(match r {
            t::ValueRange::Value(e) => (self.expr(e)?, None),
            t::ValueRange::Range(a, b) => {
                if matches!(a.kind, t::ExprKind::Dollar) || matches!(b.kind, t::ExprKind::Dollar) {
                    return self.unsupported(a.loc, "a `$` bound in a constraint range");
                }
                (self.expr(a)?, Some(self.expr(b)?))
            }
        })
    }

    /// `constraint name { items }` of a class.
    pub(crate) fn constraint_decl(&mut self, k: &t::ConstraintDecl) -> Result<Constraint> {
        if !k.quals.is_empty() {
            return self.unsupported(k.loc, &format!("a{} constraint", k.quals.iter().map(|q| format!(" {}", q.as_str())).collect::<String>()));
        }
        if !k.name.scopes.is_empty() {
            return self.unsupported(k.loc, "a constraint declared outside its class");
        }
        let Some(items) = &k.items else {
            return self.unsupported(k.loc, "a constraint prototype");
        };
        Ok(Constraint { name: k.name.name.clone(), static_: k.static_, items: self.constraint_items(items)?, line: k.loc.line })
    }
}
