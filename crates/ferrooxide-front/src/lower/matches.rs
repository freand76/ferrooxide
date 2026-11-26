//! Pattern matching (IEEE 1800-2017 12.6): `case ... matches`, `if (e matches p &&& g)` and
//! `e matches p ? a : b`. A pattern becomes tests of the tags (`TagIs`) and of the values it
//! names, run outermost first, and its pattern variables become variables of a block around
//! the code that sees them (in a conditional expression: the members they name).

use super::{Lower, SvType};
use crate::ast::*;
use crate::syntax::kw::Kw;
use crate::syntax::tree as t;
use crate::Result;
use std::rc::Rc;

/// One test of a pattern.
enum Test {
    /// the tag of the tagged union value is member `m`
    Tag(Expr, String),
    /// the value is a constant (compared as the case statement compares)
    Value(Expr, Expr),
}

/// A pattern variable: its name, type and the part of the value it takes.
struct Bind {
    name: String,
    ty: Rc<SvType>,
    sel: Expr,
}

fn assign(lhs: Expr, rhs: Expr, line: Line) -> Stmt {
    Stmt::Assign { lhs, rhs, nonblocking: false, timing: None, line }
}

fn bit(v: u64) -> Expr {
    ExprKind::Num(Number { size: Some(1), signed: false, base: 2, based: true, digits: v.to_string(), fill: false }).into()
}

fn block(decls: Vec<Decl>, stmts: Vec<Stmt>) -> Stmt {
    Stmt::Block { name: None, decls, imports: vec![], params: vec![], stmts, fork: None }
}

/// The parts of a condition `a &&& b &&& ...`, left to right.
fn cond_parts(e: &t::Expr) -> Vec<&t::Expr> {
    match &e.kind {
        t::ExprKind::TripleAnd(a, b) => {
            let mut v = cond_parts(a);
            v.extend(cond_parts(b));
            v
        }
        _ => vec![e],
    }
}

/// `e` with the pattern variables replaced by the members they name.
fn substitute(e: &mut Expr, binds: &[Bind]) {
    if let ExprKind::Ident(n) = &e.kind {
        if let Some(b) = binds.iter().find(|b| b.name == *n) {
            *e = ExprKind::Unchecked(Box::new(b.sel.clone())).into();
        }
        return;
    }
    e.for_each_child_mut(&mut |c| substitute(c, binds));
}

impl Lower {
    /// The tests and variables of pattern `p` for value `sel` of type `ty`.
    fn pattern_parts(&mut self, p: &t::Pattern, sel: Expr, ty: &Rc<SvType>, loc: t::Loc, tests: &mut Vec<Test>, binds: &mut Vec<Bind>) -> Result<()> {
        let fields = ty.fields.as_ref().filter(|_| ty.dims.is_empty() && ty.unpacked.is_empty() && ty.heap.is_none());
        match p {
            t::Pattern::Var(n) => {
                let name = self.name(n, loc);
                if binds.iter().any(|b| b.name == name) {
                    return self.err(loc, format!("pattern variable '{n}' is declared twice in the pattern"));
                }
                binds.push(Bind { name, ty: ty.clone(), sel });
            }
            t::Pattern::Wild => {}
            t::Pattern::Expr(e) => {
                let v = self.expr(e)?;
                tests.push(Test::Value(sel, v));
            }
            t::Pattern::Tagged(m, inner) => {
                let Some(f) = fields.filter(|f| f.tagged) else {
                    return self.err(loc, format!("pattern `tagged {m}` for a value that is not a tagged union"));
                };
                let Some((_, mt)) = f.fields.iter().find(|x| x.0 == *m) else {
                    return self.err(loc, format!("the tagged union has no member '{m}'"));
                };
                let mt = Rc::new(mt.clone());
                tests.push(Test::Tag(sel.clone(), m.clone()));
                if let Some(q) = inner {
                    if mt.fields.as_ref().is_some_and(|x| x.fields.is_empty() && !x.union) {
                        return self.err(loc, format!("'{m}' is a void member: `tagged {m}` takes no pattern"));
                    }
                    self.pattern_parts(q, ExprKind::Member(Box::new(sel), m.clone()).into(), &mt, loc, tests, binds)?;
                }
            }
            t::Pattern::List(items) => {
                let Some(f) = fields.filter(|f| !f.union) else {
                    return match ty.unpacked.is_empty() && ty.heap.is_none() && (ty.fields.is_none() || !ty.dims.is_empty()) {
                        false if ty.fields.is_none() => self.unsupported(loc, "a pattern of an array"),
                        _ => self.err(loc, "a structure pattern `'{...}` for a value that is not a struct"),
                    };
                };
                let named = items.iter().any(|i| i.0.is_some());
                if named && items.iter().any(|i| i.0.is_none()) {
                    return self.err(loc, "a structure pattern with both positional and named members");
                }
                if !named && items.len() != f.fields.len() {
                    return self.err(loc, format!("the structure pattern has {} members for a struct of {}", items.len(), f.fields.len()));
                }
                for (k, (n, q)) in items.iter().enumerate() {
                    let (name, mt) = match n {
                        None => f.fields[k].clone(),
                        Some(n) => match f.fields.iter().find(|x| x.0 == *n) {
                            Some(x) => x.clone(),
                            None => return self.err(loc, format!("the struct has no member '{n}'")),
                        },
                    };
                    self.pattern_parts(q, ExprKind::Member(Box::new(sel.clone()), name).into(), &Rc::new(mt), loc, tests, binds)?;
                }
            }
        }
        Ok(())
    }

    /// A variable of type `ty` (automatic: each activation has its own).
    fn match_var(&mut self, name: &str, ty: Rc<SvType>, loc: t::Loc) -> Result<Decl> {
        self.spec_count += 1;
        let tn = format!("pattern_type${}", self.spec_count);
        self.types.insert(tn.clone(), ty);
        let named = t::Name { scopes: vec![], name: tn, params: None, loc };
        let dt = t::DataType { kind: t::TypeKind::Named(Box::new(named)), signed: None, packed: vec![], loc };
        let item = t::Item::Var(t::VarDecl { const_: false, var: true, lifetime: None, ty: dt, decls: vec![t::Declarator { name: name.to_string(), dims: vec![], init: None, loc }], loc });
        let (mut decls, _) = self.block_decls(std::slice::from_ref(&item))?;
        let mut d = decls.pop().unwrap();
        d.automatic = true;
        Ok(d)
    }

    /// The type of the value a pattern matches.
    fn match_type(&mut self, e: &t::Expr) -> Result<Rc<SvType>> {
        match self.expr_type(e)? {
            Some(ty) => Ok(Rc::new(ty)),
            None => self.unsupported(e.loc, "pattern matching of this expression (only of variables)"),
        }
    }

    /// `inner` run when the tests pass, the tags tested before the members under them.
    fn tested(tests: Vec<Test>, kind: CaseKind, inner: Stmt, line: Line) -> Stmt {
        tests.into_iter().rev().fold(inner, |acc, t| match t {
            Test::Tag(e, m) => Stmt::If { cond: ExprKind::TagIs(Box::new(e), m).into(), then_s: Box::new(acc), else_s: None, line },
            Test::Value(e, v) => Stmt::Case { kind, expr: e, arms: vec![(vec![v], acc)], default: None, line },
        })
    }

    /// The pattern variables declared and set, `rest` lowered where they are visible, all in
    /// a block.
    fn with_binds(&mut self, binds: Vec<Bind>, loc: t::Loc, rest: &mut dyn FnMut(&mut Self) -> Result<Stmt>) -> Result<Stmt> {
        let saved = self.vars.clone();
        let nlocals = self.wild.locals.len();
        let r = (|| {
            let mut decls = Vec::new();
            let mut stmts = Vec::new();
            for b in binds {
                decls.push(self.match_var(&b.name, b.ty.clone(), loc)?);
                self.wild.locals.push(b.name.clone());
                stmts.push(assign(ExprKind::Ident(b.name).into(), b.sel, loc.line));
            }
            stmts.push(rest(self)?);
            Ok(block(decls, stmts))
        })();
        self.wild.locals.truncate(nlocals);
        self.vars = saved;
        r
    }

    /// The parts of an `if` condition from `parts` on: each `e matches p` tests `e` (copied
    /// into a variable) against `p`, each other part is a condition; when all hold, `flag` is
    /// set and `then` runs.
    fn if_chain(&mut self, parts: &[&t::Expr], then: &t::Stmt, flag: &str, loc: t::Loc) -> Result<Stmt> {
        let line = loc.line;
        let Some((first, rest)) = parts.split_first() else {
            let body = self.stmt(then)?;
            return Ok(block(vec![], vec![assign(ExprKind::Ident(flag.into()).into(), bit(1), line), body]));
        };
        let t::ExprKind::Matches(e, p) = &first.kind else {
            let cond = self.expr(first)?;
            let inner = self.if_chain(rest, then, flag, loc)?;
            return Ok(Stmt::If { cond, then_s: Box::new(inner), else_s: None, line });
        };
        let ty = self.match_type(e)?;
        let v = self.expr(e)?;
        self.spec_count += 1;
        let tmp = format!("matches${}", self.spec_count);
        let saved = self.vars.clone();
        let d = self.match_var(&tmp, ty.clone(), loc)?;
        let (mut tests, mut binds) = (Vec::new(), Vec::new());
        self.pattern_parts(p, ExprKind::Ident(tmp.clone()).into(), &ty, loc, &mut tests, &mut binds)?;
        let inner = self.with_binds(binds, loc, &mut |l| l.if_chain(rest, then, flag, loc));
        self.vars = saved;
        let inner = Self::tested(tests, CaseKind::Case, inner?, line);
        Ok(block(vec![d], vec![assign(ExprKind::Ident(tmp).into(), v, line), inner]))
    }

    /// `if (cond) then else` with a pattern or `&&&` in `cond`.
    pub(crate) fn if_matches(&mut self, cond: &t::Expr, then: &t::Stmt, else_: Option<&t::Stmt>, loc: t::Loc) -> Result<Stmt> {
        let line = loc.line;
        self.spec_count += 1;
        let flag = format!("matched${}", self.spec_count);
        let fd = self.match_var(&flag, Rc::new(SvType::vector(NetKind::Bit, false, vec![])), loc)?;
        let chain = self.if_chain(&cond_parts(cond), then, &flag, loc)?;
        let mut stmts = vec![assign(ExprKind::Ident(flag.clone()).into(), bit(0), line), chain];
        if let Some(e) = else_ {
            let e = self.stmt(e)?;
            stmts.push(Stmt::If { cond: ExprKind::Unary(UnOp::LNot, Box::new(ExprKind::Ident(flag).into())).into(), then_s: Box::new(e), else_s: None, line });
        }
        Ok(block(vec![fd], stmts))
    }

    /// `case (e) matches` (`casez`, `casex`): the first item whose pattern matches and whose
    /// guard holds runs, else the default item.
    pub(crate) fn case_matches(&mut self, kind: CaseKind, qual: Option<Kw>, e: &t::Expr, items: &[t::CaseItem], loc: t::Loc) -> Result<Stmt> {
        let line = loc.line;
        let ty = self.match_type(e)?;
        let v = self.expr(e)?;
        self.spec_count += 1;
        let (tmp, flag) = (format!("matches${}", self.spec_count), format!("matched${}", self.spec_count));
        let saved = self.vars.clone();
        let decls = vec![self.match_var(&tmp, ty.clone(), loc)?, self.match_var(&flag, Rc::new(SvType::vector(NetKind::Bit, false, vec![])), loc)?];
        let mut stmts = vec![assign(ExprKind::Ident(tmp.clone()).into(), v, line), assign(ExprKind::Ident(flag.clone()).into(), bit(0), line)];
        let not_yet = || -> Expr { ExprKind::Unary(UnOp::LNot, Box::new(ExprKind::Ident(flag.clone()).into())).into() };
        let mut default = None;
        let r = (|| {
            for it in items {
                let Some(p) = &it.pattern else {
                    default = Some(self.stmt(&it.body)?);
                    continue;
                };
                let (mut tests, mut binds) = (Vec::new(), Vec::new());
                self.pattern_parts(p, ExprKind::Ident(tmp.clone()).into(), &ty, it.loc, &mut tests, &mut binds)?;
                let arm = self.with_binds(binds, it.loc, &mut |l| {
                    let body = l.stmt(&it.body)?;
                    let hit = block(vec![], vec![assign(ExprKind::Ident(flag.clone()).into(), bit(1), it.loc.line), body]);
                    Ok(match &it.guard {
                        Some(g) => {
                            let g = cond_parts(g);
                            if g.iter().any(|x| matches!(x.kind, t::ExprKind::Matches(..))) {
                                return l.unsupported(it.loc, "a pattern in the guard of a case item");
                            }
                            let mut cond = l.expr(g[0])?;
                            for x in &g[1..] {
                                cond = ExprKind::Binary(BinOp::LAnd, Box::new(cond), Box::new(l.expr(x)?)).into();
                            }
                            Stmt::If { cond, then_s: Box::new(hit), else_s: None, line: it.loc.line }
                        }
                        None => hit,
                    })
                })?;
                let arm = Self::tested(tests, kind, arm, it.loc.line);
                stmts.push(Stmt::If { cond: not_yet(), then_s: Box::new(arm), else_s: None, line: it.loc.line });
            }
            Ok(())
        })();
        self.vars = saved;
        r?;
        // a value no item matches is reported at run time (not for unique0)
        if default.is_none() && matches!(qual, Some(Kw::Unique | Kw::Priority)) {
            default = Some(Stmt::SysTask { name: "$__unhandled_case".into(), args: Vec::new(), line });
        }
        if let Some(d) = default {
            stmts.push(Stmt::If { cond: not_yet(), then_s: Box::new(d), else_s: None, line });
        }
        Ok(block(decls, stmts))
    }

    /// `cond ? a : b` with a pattern or `&&&` in `cond`: the tests as one condition (`===`
    /// for the values), the pattern variables in `a` replaced by the members they name.
    pub(crate) fn cond_matches(&mut self, cond: &t::Expr, a: &t::Expr, b: &t::Expr, loc: t::Loc) -> Result<Expr> {
        let mut conds: Vec<Expr> = Vec::new();
        let mut binds = Vec::new();
        for part in cond_parts(cond) {
            let t::ExprKind::Matches(e, p) = &part.kind else {
                let mut c = self.expr(part)?;
                substitute(&mut c, &binds);
                conds.push(c);
                continue;
            };
            if super::stmt::syn_side_effects(e) {
                return self.unsupported(e.loc, "pattern matching of an expression with side effects in a conditional expression");
            }
            let ty = self.match_type(e)?;
            let mut v = self.expr(e)?;
            substitute(&mut v, &binds);
            let mut tests = Vec::new();
            self.pattern_parts(p, v, &ty, loc, &mut tests, &mut binds)?;
            for t in tests {
                conds.push(match t {
                    Test::Tag(e, m) => ExprKind::TagIs(Box::new(e), m).into(),
                    Test::Value(e, v) => ExprKind::Binary(BinOp::CaseEq, Box::new(ExprKind::Unchecked(Box::new(e)).into()), Box::new(v)).into(),
                });
            }
        }
        let cond = conds.into_iter().reduce(|x, y| ExprKind::Binary(BinOp::LAnd, Box::new(x), Box::new(y)).into()).unwrap();
        let saved = self.vars.clone();
        for bd in &binds {
            self.vars.insert(bd.name.clone(), (bd.ty.clone(), vec![]));
        }
        let ra = self.expr(a);
        self.vars = saved;
        let mut a = ra?;
        substitute(&mut a, &binds);
        let b = self.expr(b)?;
        Ok(ExprKind::Cond(Box::new(cond), Box::new(a), Box::new(b)).into())
    }
}
