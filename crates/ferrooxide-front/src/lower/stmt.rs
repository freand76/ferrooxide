//! Statements, with the expansions the compiler expects: foreach as nested for loops,
//! `case inside` as `case (1'b1)`, unique/priority checks, `@(e iff g)`, `x++`, compound
//! assignments.

use super::{item_loc, item_what, Lower};
use crate::ast::*;

use crate::syntax::kw::Kw;
use crate::syntax::tree as t;
use crate::Result;

pub(crate) fn assign_binop(op: t::AssignOp) -> Option<BinOp> {
    use t::AssignOp as A;
    Some(match op {
        A::Eq => return None,
        A::Add => BinOp::Add,
        A::Sub => BinOp::Sub,
        A::Mul => BinOp::Mul,
        A::Div => BinOp::Div,
        A::Mod => BinOp::Mod,
        A::And => BinOp::And,
        A::Or => BinOp::Or,
        A::Xor => BinOp::Xor,
        A::Shl => BinOp::Shl,
        A::Shr => BinOp::Shr,
        A::AShl => BinOp::AShl,
        A::AShr => BinOp::AShr,
    })
}

/// Whether evaluating an expression assigns something (an increment in it).
fn side_effects(e: &Expr) -> bool {
    match e {
        Expr::IncDec { .. } => true,
        Expr::Index(a, b) | Expr::Binary(_, a, b) => side_effects(a) || side_effects(b),
        Expr::Range(a, b, c) | Expr::IndexedRange(a, b, c, _) | Expr::Cond(a, b, c) => side_effects(a) || side_effects(b) || side_effects(c),
        Expr::Unary(_, a) | Expr::Member(a, _) => side_effects(a),
        Expr::Concat(v) | Expr::Call(_, v) | Expr::SysCall(_, v) => v.iter().any(side_effects),
        _ => false,
    }
}

fn block(stmts: Vec<Stmt>) -> Stmt {
    Stmt::Block { name: None, decls: vec![], stmts, fork: None }
}

impl Lower {
    /// An immediate assertion (IEEE 1800-2017 16.3): `assert`/`assume` run the pass action
    /// when the expression is true, else the fail action (by default `$error`). `cover` does
    /// nothing, as in the reference (ICARUS_NOTES 11). A label names a scope around it (for
    /// `%m`).
    fn immediate_assertion(&mut self, a: &t::Assertion, label: Option<&String>) -> Result<Stmt> {
        let line = a.loc.line;
        let expr = match &a.body {
            t::AssertBody::Immediate { deferred: None, expr } => expr,
            t::AssertBody::Immediate { .. } => return self.unsupported(a.loc, "a deferred assertion"),
            _ => return self.unsupported(a.loc, "a concurrent assertion"),
        };
        if a.kind == t::AssertKind::Restrict {
            return self.unsupported(a.loc, "an immediate restrict");
        }
        if a.kind == t::AssertKind::Cover {
            return Ok(Stmt::Null);
        }
        let cond = self.expr(expr)?;
        let pass = match &a.action.pass {
            Some(st) => self.stmt(st)?,
            None => Stmt::Null,
        };
        let fail = match &a.action.fail {
            Some(st) => Some(self.stmt(st)?),
            None => Some(Stmt::SysTask { name: "$error".into(), args: Vec::new(), line }),
        };
        let st = Stmt::If { cond, then_s: Box::new(pass), else_s: fail.map(Box::new), line };
        Ok(match label {
            Some(l) => Stmt::Block { name: Some(self.name(l, a.loc)), decls: vec![], stmts: vec![st], fork: None },
            None => st,
        })
    }

    /// The declarations of a block (no directions).
    fn block_decls(&mut self, v: &[t::Item]) -> Result<Vec<Decl>> {
        let mut decls = Vec::new();
        for it in v {
            let (net, var, ty, dl, strength, delay, loc) = match it {
                t::Item::Var(v) => {
                    if v.const_ {
                        return self.unsupported(v.loc, "a const variable");
                    }
                    if v.lifetime == Some(t::Lifetime::Automatic) || (v.lifetime.is_some() && self.in_subroutine) {
                        return self.unsupported(v.loc, "an automatic variable declaration");
                    }
                    (None, v.var, &v.ty, &v.decls, None, None, v.loc)
                }
                t::Item::Net(n) if n.user.is_none() => (n.net, false, &n.ty, &n.decls, n.strength.as_ref(), n.delay.as_ref(), n.loc),
                t::Item::Param(p) => return self.err(p.loc, "parameters in blocks are not supported"),
                other => return self.unsupported(item_loc(other), &format!("{} in a block", item_what(other))),
            };
            let mut d = self.decl_head(None, net, var, ty, strength, delay, loc)?;
            if !d.explicit_kind {
                d.kind = NetKind::Reg;
            }
            d.names = self.declarators(dl)?;
            decls.push(d);
        }
        Ok(decls)
    }

    pub(crate) fn stmt(&mut self, s: &t::Stmt) -> Result<Stmt> {
        let line = s.loc.line;
        if s.label.is_some() && !matches!(s.kind, t::StmtKind::Block { .. } | t::StmtKind::Assertion(_)) {
            return self.unsupported(s.loc, "a statement label");
        }
        Ok(match &s.kind {
            t::StmtKind::Null => Stmt::Null,
            t::StmtKind::Block { fork, name, decls, stmts } => {
                let name = match (name, &s.label) {
                    (Some(n), _) | (None, Some(n)) => Some(self.name(n, s.loc)),
                    _ => None,
                };
                let decls = self.block_decls(decls)?;
                let mut v = Vec::new();
                for st in stmts {
                    v.push(self.stmt(st)?);
                }
                let fork = fork.map(|j| match j {
                    t::Join::All => Join::All,
                    t::Join::Any => Join::Any,
                    t::Join::None => Join::None,
                });
                Stmt::Block { name, decls, stmts: v, fork }
            }
            t::StmtKind::Timed { timing, body } => match timing {
                t::Timing::Delay(d) => {
                    let d = self.delay(d)?;
                    let body = self.stmt(body)?;
                    Stmt::Timed { timing: Timing::Delay(d), body: Box::new(body), line }
                }
                t::Timing::Event(ec) => {
                    let (ec, guard) = self.event_ctl_iff(ec, s.loc)?;
                    let body = self.stmt(body)?;
                    let Some(g) = guard else {
                        return Ok(Stmt::Timed { timing: Timing::Event(ec), body: Box::new(body), line });
                    };
                    // @(e iff g) s: wait again while g is false when e happens
                    let again = Stmt::While {
                        cond: Expr::Unary(UnOp::LNot, Box::new(g)),
                        body: Box::new(Stmt::Timed { timing: Timing::Event(ec.clone()), body: Box::new(Stmt::Null), line }),
                        line,
                    };
                    Stmt::Timed { timing: Timing::Event(ec), body: Box::new(block(vec![again, body])), line }
                }
                t::Timing::Cycle(_) => return self.unsupported(s.loc, "a cycle delay"),
                t::Timing::Repeat(..) => return self.unsupported(s.loc, "repeat as a statement's timing control"),
            },
            t::StmtKind::Trigger { nonblocking: false, timing: None, target } => {
                let Some(p) = self.dotted(target)? else { return self.unsupported(s.loc, "this event trigger") };
                Stmt::Trigger { name: p, line }
            }
            t::StmtKind::Trigger { .. } => return self.unsupported(s.loc, "a nonblocking event trigger"),
            t::StmtKind::If { qual, cond, then_, else_ } => {
                if matches!(cond.kind, t::ExprKind::Matches(..) | t::ExprKind::TripleAnd(..)) {
                    return self.unsupported(cond.loc, "a pattern-matching condition");
                }
                let cond = self.expr(cond)?;
                let then_s = Box::new(self.stmt(then_)?);
                let else_s = match else_ {
                    Some(e) => Some(Box::new(self.stmt(e)?)),
                    None => None,
                };
                let mut st = Stmt::If { cond, then_s, else_s, line };
                if matches!(qual, Some(Kw::Unique | Kw::Priority)) {
                    // an if / else-if chain without a final else (IEEE 1800 12.4.2)
                    fn last_else(s: &mut Stmt, line: Line) {
                        if let Stmt::If { else_s, .. } = s {
                            match else_s {
                                None => *else_s = Some(Box::new(Stmt::SysTask { name: "$__unhandled_if".into(), args: Vec::new(), line })),
                                Some(e) => last_else(e, line),
                            }
                        }
                    }
                    last_else(&mut st, line);
                }
                st
            }
            t::StmtKind::Case { qual, kw, expr, mode, items } => {
                let kind = match kw {
                    t::CaseKw::Case => CaseKind::Case,
                    t::CaseKw::Casez => CaseKind::CaseZ,
                    t::CaseKw::Casex => CaseKind::CaseX,
                };
                let mut e = self.expr(expr)?;
                let sel = match mode {
                    t::CaseMode::Normal => None,
                    t::CaseMode::Inside if kind == CaseKind::Case => {
                        // an item matches when `e inside {item}`: `case (1'b1)` with the tests
                        let one = Expr::Num(Number { size: Some(1), signed: false, base: 2, based: true, digits: "1".into(), fill: false });
                        Some(std::mem::replace(&mut e, one))
                    }
                    t::CaseMode::Inside => return self.unsupported(s.loc, "casez/casex inside"),
                    t::CaseMode::Matches => return self.unsupported(s.loc, "case matches"),
                };
                let mut arms = Vec::new();
                let mut default = None;
                for it in items {
                    if it.labels.is_empty() {
                        default = Some(Box::new(self.stmt(&it.body)?));
                        continue;
                    }
                    let mut labels = Vec::new();
                    for l in &it.labels {
                        labels.push(match (&sel, l) {
                            (Some(e), l) => self.set_item(e, l)?,
                            (None, t::ValueRange::Value(v)) => self.expr(v)?,
                            (None, t::ValueRange::Range(..)) => return self.unsupported(it.loc, "a range label outside case inside"),
                        });
                    }
                    let st = self.stmt(&it.body)?;
                    arms.push((labels, st));
                }
                // a value no item matches is reported at run time (not for unique0)
                if matches!(qual, Some(Kw::Unique | Kw::Priority)) && default.is_none() {
                    default = Some(Box::new(Stmt::SysTask { name: "$__unhandled_case".into(), args: Vec::new(), line }));
                }
                Stmt::Case { kind, expr: e, arms, default, line }
            }
            t::StmtKind::For { init, cond, step, body } => {
                let mut decls: Vec<Decl> = Vec::new();
                let mut inits = Vec::new();
                for fi in init {
                    match fi {
                        t::ForInit::Decl { var, ty, name, init, loc } => {
                            if let Some(ty) = ty {
                                let mut d = self.decl_head(None, None, *var, ty, None, None, *loc)?;
                                d.names.clear();
                                decls.push(d);
                            } else if *var {
                                let ty = t::DataType { kind: t::TypeKind::Implicit, signed: None, packed: vec![], loc: *loc };
                                let mut d = self.decl_head(None, None, true, &ty, None, None, *loc)?;
                                d.names.clear();
                                decls.push(d);
                            }
                            let name = self.name(name, *loc);
                            let v = self.expr(init)?;
                            let Some(d) = decls.last_mut() else { return self.unsupported(*loc, "this for-loop initialization") };
                            d.names.push(Declarator { name: name.clone(), dims: vec![], heap: None, init: None, line: loc.line });
                            inits.push(Stmt::Assign { lhs: Expr::Ident(name), rhs: v, nonblocking: false, timing: None, line: loc.line });
                        }
                        t::ForInit::Assign(st) => inits.push(self.simple_assign(st)?),
                    }
                }
                let cond = match cond {
                    Some(c) => self.expr(c)?,
                    None => Expr::Num(dec(1)),
                };
                let mut steps = Vec::new();
                for st in step {
                    steps.push(self.simple_assign(st)?);
                }
                let body = self.stmt(body)?;
                let one = |mut v: Vec<Stmt>| if v.len() == 1 { v.pop().unwrap() } else { block(v) };
                let f = Stmt::For { init: Box::new(one(inits)), cond, step: Box::new(one(steps)), body: Box::new(body), line };
                if decls.is_empty() {
                    f
                } else {
                    Stmt::Block { name: None, decls, stmts: vec![f], fork: None }
                }
            }
            t::StmtKind::Foreach { array, vars, body } => self.foreach(array, vars, body, s.loc)?,
            t::StmtKind::While { cond, body } => {
                let cond = self.expr(cond)?;
                Stmt::While { cond, body: Box::new(self.stmt(body)?), line }
            }
            t::StmtKind::DoWhile { body, cond } => {
                let body = Box::new(self.stmt(body)?);
                let cond = self.expr(cond)?;
                Stmt::DoWhile { cond, body, line }
            }
            t::StmtKind::Repeat { count, body } => {
                let count = self.expr(count)?;
                Stmt::Repeat { count, body: Box::new(self.stmt(body)?), line }
            }
            t::StmtKind::Forever(b) => Stmt::Forever { body: Box::new(self.stmt(b)?), line },
            t::StmtKind::Wait { cond, body } => {
                let cond = self.expr(cond)?;
                Stmt::Wait { cond, body: Box::new(self.stmt(body)?), line }
            }
            t::StmtKind::ProcCont { kw, lhs, rhs } => {
                let lhs = self.lvalue(lhs)?;
                let rhs = match rhs {
                    Some(r) if matches!(kw, Kw::Force | Kw::Assign) => Some(self.expr(r)?),
                    _ => None,
                };
                Stmt::Force { lhs, rhs, line }
            }
            t::StmtKind::Disable(e) => {
                let Some(p) = self.dotted(e)? else { return self.unsupported(s.loc, "this disable target") };
                Stmt::Disable { name: p, line }
            }
            t::StmtKind::DisableFork => Stmt::Disable { name: "fork".into(), line },
            t::StmtKind::Return(v) => {
                let value = match v {
                    Some(e) => Some(self.expr(e)?),
                    None => None,
                };
                Stmt::Return { value, line }
            }
            t::StmtKind::Break => Stmt::Break { line },
            t::StmtKind::Continue => Stmt::Continue { line },
            t::StmtKind::Expr(e) => self.expr_stmt(e, s.loc)?,
            t::StmtKind::Assign { .. } | t::StmtKind::NonBlocking { .. } => {
                // (the old parser read a statement's leading name twice: segments too)
                let lhs = match &s.kind {
                    t::StmtKind::Assign { lhs, .. } | t::StmtKind::NonBlocking { lhs, .. } => lhs,
                    _ => unreachable!(),
                };
                if !matches!(lhs.kind, t::ExprKind::Concat(_)) {
                    self.hier_path(lhs)?;
                }
                self.simple_assign(s)?
            }
            t::StmtKind::WaitFork => return self.unsupported(s.loc, "wait fork"),
            t::StmtKind::WaitOrder { .. } => return self.unsupported(s.loc, "wait_order"),
            t::StmtKind::RandCase(_) => return self.unsupported(s.loc, "randcase"),
            t::StmtKind::RandSequence { .. } => return self.unsupported(s.loc, "randsequence"),
            t::StmtKind::Assertion(a) => self.immediate_assertion(a, s.label.as_ref().or(a.label.as_ref()))?,
            // `expect(e);` in Verilog code: a call of a task named `expect`
            t::StmtKind::Expect { prop: t::Prop::Expr(e), action } if action.pass.is_none() && action.fail.is_none() => {
                let e = self.arg(e)?;
                Stmt::TaskCall { name: self.name("expect", s.loc), args: vec![e], line }
            }
            t::StmtKind::Expect { .. } => return self.unsupported(s.loc, "expect"),
            t::StmtKind::Item(_) => return self.unsupported(s.loc, "a declaration in a statement"),
        })
    }

    /// An expression used as a statement: a task call, `x++`.
    fn expr_stmt(&mut self, e: &t::Expr, loc: t::Loc) -> Result<Stmt> {
        let line = loc.line;
        match &e.kind {
            t::ExprKind::SysCall { name, args, .. } => {
                let mut v = Vec::new();
                for a in args {
                    v.push(match a {
                        t::Arg::Pos(None) => None,
                        t::Arg::Pos(Some(x)) => Some(self.arg(x)?),
                        t::Arg::Named(..) => return self.unsupported(loc, "a named argument"),
                    });
                }
                Ok(Stmt::SysTask { name: name.clone(), args: v, line })
            }
            t::ExprKind::IncDec { .. } => {
                if let t::ExprKind::IncDec { prefix: false, e: x, .. } = &e.kind {
                    if !matches!(x.kind, t::ExprKind::Concat(_)) {
                        self.hier_path(x)?;
                    }
                }
                self.simple_assign(&t::Stmt { kind: t::StmtKind::Expr(e.clone()), label: None, loc })
            }
            t::ExprKind::Call { with_: None, .. } | t::ExprKind::Member(..) if self.class_call(e) => match self.expr(e)? {
                Expr::MethodCall(obj, name, args) => Ok(Stmt::MethodCall { obj: *obj, name, args, line }),
                Expr::Member(obj, name) => Ok(Stmt::MethodCall { obj: *obj, name, args: vec![], line }),
                _ => self.unsupported(loc, "this expression as a statement"),
            },
            t::ExprKind::Call { func, args, with_: None, .. } => {
                let name = self.task_name(func, loc)?;
                let args = self.call_args(args, loc)?;
                Ok(Stmt::TaskCall { name, args, line })
            }
            t::ExprKind::Ident(_) | t::ExprKind::Member(..) | t::ExprKind::Index(..) | t::ExprKind::Scoped(_) => {
                let name = self.task_name(e, loc)?;
                Ok(Stmt::TaskCall { name, args: vec![], line })
            }
            t::ExprKind::Cast(t::CastTarget::Type(ty), x) if matches!(ty.kind, t::TypeKind::Void) => {
                if !matches!(x.kind, t::ExprKind::Call { .. } | t::ExprKind::SysCall { .. } | t::ExprKind::Member(..)) {
                    return self.err(loc, "a void cast needs a function call");
                }
                Ok(Stmt::Discard { e: self.expr(x)?, line })
            }
            _ => self.unsupported(loc, "this expression as a statement"),
        }
    }

    /// A call through a class handle (`obj.m(..)`, `this.m`, `super.new(..)`).
    fn class_call(&self, e: &t::Expr) -> bool {
        let mut cur = e;
        loop {
            match &cur.kind {
                t::ExprKind::Call { func, .. } => cur = func,
                t::ExprKind::Member(a, _) | t::ExprKind::Index(a, _) => cur = a,
                t::ExprKind::This | t::ExprKind::Super => return true,
                t::ExprKind::Ident(n) => return !std::ptr::eq(cur, e) && self.is_class_var(n),
                _ => return false,
            }
        }
    }

    /// The name of a called task: `t`, `a.b.t`, `blk[1].t`, `pkg::t`.
    fn task_name(&mut self, f: &t::Expr, loc: t::Loc) -> Result<String> {
        if let t::ExprKind::Scoped(n) = &f.kind {
            if n.scopes.len() == 1 && n.params.is_none() && n.scopes[0].params.is_none() {
                let p = &n.scopes[0].name;
                if let Some(pk) = self.packages.get(p).cloned() {
                    if pk.funcs.contains(&n.name) {
                        self.use_package(p);
                        return Ok(format!("{p}.{}", n.name));
                    }
                    if let Some(t) = pk.types.get(&n.name) {
                        self.types.insert(n.name.clone(), t.clone());
                    }
                    self.use_package(p);
                    return Ok(n.name.clone());
                }
            }
            return self.unsupported(loc, "this scoped task name");
        }
        match self.hier_path(f)? {
            Some(p) => Ok(p.join(".")),
            None => self.unsupported(loc, "this task call"),
        }
    }

    /// `a.b.c` (no indexes) for triggers and disable.
    fn dotted(&mut self, e: &t::Expr) -> Result<Option<String>> {
        fn go(l: &mut Lower, e: &t::Expr, out: &mut Vec<String>) -> bool {
            match &e.kind {
                t::ExprKind::Ident(n) => {
                    out.push(l.name(n, e.loc));
                    true
                }
                t::ExprKind::Member(a, m) => {
                    if matches!(a.kind, t::ExprKind::Root) {
                        out.push(l.name(m, e.loc));
                        return true;
                    }
                    let ok = go(l, a, out);
                    out.push(m.clone());
                    ok
                }
                _ => false,
            }
        }
        let mut v = Vec::new();
        Ok(go(self, e, &mut v).then(|| v.join(".")))
    }

    /// An assignment without terminator: `lhs = e`, `lhs <= #d e`, `lhs op= e`, `x++`, `++x`.
    pub(crate) fn simple_assign(&mut self, s: &t::Stmt) -> Result<Stmt> {
        let line = s.loc.line;
        match &s.kind {
            t::StmtKind::Expr(t::Expr { kind: t::ExprKind::IncDec { inc, e, .. }, .. }) => {
                let lhs = self.lvalue(e)?;
                if side_effects(&lhs) {
                    return self.unsupported(s.loc, "an increment of a target with side effects");
                }
                let op = if *inc { BinOp::Add } else { BinOp::Sub };
                let rhs = Expr::Binary(op, Box::new(lhs.clone()), Box::new(Expr::Num(dec(1))));
                Ok(Stmt::Assign { lhs, rhs, nonblocking: false, timing: None, line })
            }
            t::StmtKind::Assign { lhs, op, timing, rhs } if *op != t::AssignOp::Eq => {
                if timing.is_some() {
                    return self.unsupported(s.loc, "a timing control in a compound assignment");
                }
                let lhs = self.lvalue(lhs)?;
                if side_effects(&lhs) {
                    return self.unsupported(s.loc, "a compound assignment to a target with side effects");
                }
                let r = self.expr(rhs)?;
                let rhs = Expr::Binary(assign_binop(*op).unwrap(), Box::new(lhs.clone()), Box::new(r));
                Ok(Stmt::Assign { lhs, rhs, nonblocking: false, timing: None, line })
            }
            t::StmtKind::Assign { lhs, timing, rhs, .. } | t::StmtKind::NonBlocking { lhs, timing, rhs } => {
                let nonblocking = matches!(s.kind, t::StmtKind::NonBlocking { .. });
                let lhs = self.lvalue(lhs)?;
                let timing = match timing {
                    Some(tm) => Some(self.assign_timing(tm, s.loc)?),
                    None => None,
                };
                let rhs = self.expr(rhs)?;
                Ok(Stmt::Assign { lhs, rhs, nonblocking, timing, line })
            }
            _ => self.unsupported(s.loc, "this statement here"),
        }
    }

    /// The timing control of an assignment.
    fn assign_timing(&mut self, tm: &t::Timing, loc: t::Loc) -> Result<Timing> {
        Ok(match tm {
            t::Timing::Delay(d) => Timing::Delay(self.delay(d)?),
            t::Timing::Event(ec) => Timing::Event(self.event_ctl(ec, loc)?),
            t::Timing::Repeat(n, ec) => {
                let n = self.expr(n)?;
                Timing::Repeat(n, self.event_ctl(ec, loc)?)
            }
            t::Timing::Cycle(_) => return self.unsupported(loc, "a cycle delay"),
        })
    }

    fn event_ctl(&mut self, ec: &t::EventCtl, loc: t::Loc) -> Result<EventCtl> {
        let (ec, guard) = self.event_ctl_iff(ec, loc)?;
        if guard.is_some() {
            return self.err(loc, "`iff` is only supported in an event control before a statement");
        }
        Ok(ec)
    }

    /// An event control with an optional `iff` condition, the same for every item.
    fn event_ctl_iff(&mut self, ec: &t::EventCtl, loc: t::Loc) -> Result<(EventCtl, Option<Expr>)> {
        match ec {
            t::EventCtl::Star => Ok((EventCtl::Star, None)),
            t::EventCtl::Expr(_) => self.unsupported(loc, "this event control"),
            t::EventCtl::List(items) => {
                let mut v = Vec::new();
                let mut guards = Vec::new();
                for it in items {
                    let edge = match it.edge {
                        None => Edge::Any,
                        Some(t::Edge::Pos) => Edge::Pos,
                        Some(t::Edge::Neg) => Edge::Neg,
                        Some(t::Edge::Both) => return self.unsupported(it.loc, "an `edge` event"),
                    };
                    let e = self.expr(&it.expr)?;
                    v.push((edge, e));
                    guards.push(match &it.iff {
                        Some(g) => Some(self.expr(g)?),
                        None => None,
                    });
                }
                let guard = match guards.first() {
                    Some(Some(g)) if guards.iter().all(|x| x.as_ref().is_some_and(|x| format!("{x:?}") == format!("{g:?}"))) => Some(g.clone()),
                    _ if guards.iter().all(|x| x.is_none()) => None,
                    _ => return self.err(loc, "`iff` on only some items of an event control is not supported"),
                };
                Ok((EventCtl::List(v), guard))
            }
        }
    }

    /// foreach (a[i, , k]) body: nested loops from the left to the right bound of each named
    /// dimension, with int loop variables.
    /// `foreach (aa[k])` of an associative array: `k` of the key type through the keys in
    /// order (`for (ok = aa.first(k); ok; ok = aa.next(k))`).
    fn foreach_assoc(&mut self, name: &str, v: &str, key: Option<&super::SvType>, body: Stmt, line: u32) -> Stmt {
        let ok = format!("{v}$ok");
        let call = |m: &str| Expr::Call(format!("{name}.{m}"), vec![Expr::Ident(v.to_string())]);
        let set = |rhs: Expr| Stmt::Assign { lhs: Expr::Ident(ok.clone()), rhs, nonblocking: false, timing: None, line };
        let lp = Stmt::For {
            init: Box::new(set(call("first"))),
            cond: Expr::Ident(ok.clone()),
            step: Box::new(set(call("next"))),
            body: Box::new(body),
            line,
        };
        let decl = |kind: NetKind, signed: bool, range: Option<Range>, n: &str| Decl {
            dir: None,
            kind,
            net: NetType::Wire,
            explicit_kind: true,
            real_net: false,
            strength: None,
            delay: None,
            signed,
            range,
            packed: vec![],
            type_param: None,
            ty: None,
            names: vec![Declarator { name: n.to_string(), dims: vec![], heap: None, init: None, line }],
            line,
        };
        // the loop variable: of the key type (`[*]`: 64-bit)
        let kd = match key {
            Some(k) if k.kind == NetKind::String => decl(NetKind::String, false, None, v),
            Some(k) => {
                let (kind, signed, range, _) = k.decl_parts();
                let range = range.or_else(|| Some((super::types::minus_one(k.width()), Expr::Num(dec(0)))));
                decl(if kind == NetKind::Bit { NetKind::Bit } else { NetKind::Reg }, signed, range, v)
            }
            None => decl(NetKind::Reg, false, Some((Expr::Num(dec(63)), Expr::Num(dec(0)))), v),
        };
        Stmt::Block { name: None, decls: vec![kd, decl(NetKind::Int(32), true, None, &ok)], stmts: vec![lp], fork: None }
    }

    /// `foreach (q[i])` of a queue or dynamic array: `i` from 0 to `q.size() - 1`.
    fn foreach_heap(&mut self, name: &str, vars: &[Option<String>], body: &t::Stmt, loc: t::Loc) -> Result<Stmt> {
        let line = loc.line;
        let body = self.stmt(body)?;
        let v = match vars {
            [Some(v)] => self.name(v, loc),
            [None] | [] => return Ok(Stmt::Null),
            _ => return self.err(loc, format!("foreach: '{name}' has only 1 dimension")),
        };
        let heap = self.vars[name].0.heap.clone();
        if let Some(HeapKind::Assoc(key)) = heap {
            return Ok(self.foreach_assoc(name, &v, key.as_deref(), body, line));
        }
        let var = || Box::new(Expr::Ident(v.clone()));
        let size = Box::new(Expr::Call(format!("{name}.size"), vec![]));
        let lp = Stmt::For {
            init: Box::new(Stmt::Assign { lhs: Expr::Ident(v.clone()), rhs: Expr::Num(dec(0)), nonblocking: false, timing: None, line }),
            cond: Expr::Binary(BinOp::Lt, var(), size),
            step: Box::new(Stmt::Assign {
                lhs: Expr::Ident(v.clone()),
                rhs: Expr::Binary(BinOp::Add, var(), Box::new(Expr::Num(dec(1)))),
                nonblocking: false,
                timing: None,
                line,
            }),
            body: Box::new(body),
            line,
        };
        let d = Decl {
            dir: None,
            kind: NetKind::Int(32),
            net: NetType::Wire,
            explicit_kind: true,
            real_net: false,
            strength: None,
            delay: None,
            signed: true,
            range: None,
            packed: vec![],
            type_param: None,
            ty: None,
            names: vec![Declarator { name: v.clone(), dims: vec![], heap: None, init: None, line }],
            line,
        };
        Ok(Stmt::Block { name: None, decls: vec![d], stmts: vec![lp], fork: None })
    }

    fn foreach(&mut self, array: &t::Expr, vars: &[Option<String>], body: &t::Stmt, loc: t::Loc) -> Result<Stmt> {
        let line = loc.line;
        let t::ExprKind::Ident(name) = &array.kind else {
            return self.err(loc, "foreach over a hierarchical name is not supported");
        };
        let name = self.name(name, array.loc);
        if self.vars.get(&name).is_some_and(|v| v.0.heap.is_some()) {
            return self.foreach_heap(&name, vars, body, loc);
        }
        let Some(dims) = self.var_dims(&name) else {
            return self.err(loc, format!("foreach: '{name}' is not an array declared here"));
        };
        let vars: Vec<Option<String>> = vars.iter().map(|v| v.as_ref().map(|n| self.name(n, loc))).collect();
        if vars.len() > dims.len() {
            return self.err(loc, format!("foreach: '{name}' has only {} dimensions", dims.len()));
        }
        let mut body = self.stmt(body)?;
        if vars.iter().all(|v| v.is_none()) {
            // no loop variables: no iterations (as the reference)
            return Ok(Stmt::Null);
        }
        let mut names = Vec::new();
        for (d, v) in vars.iter().enumerate().rev() {
            let Some(v) = v else { continue };
            let (l, r) = dims[d].clone();
            let var = || Box::new(Expr::Ident(v.clone()));
            let down = Expr::Binary(BinOp::Ge, Box::new(l.clone()), Box::new(r.clone()));
            let cond = Expr::Cond(
                Box::new(down.clone()),
                Box::new(Expr::Binary(BinOp::Ge, var(), Box::new(r.clone()))),
                Box::new(Expr::Binary(BinOp::Le, var(), Box::new(r))),
            );
            let step = Expr::Cond(
                Box::new(down),
                Box::new(Expr::Binary(BinOp::Sub, var(), Box::new(Expr::Num(dec(1))))),
                Box::new(Expr::Binary(BinOp::Add, var(), Box::new(Expr::Num(dec(1))))),
            );
            body = Stmt::For {
                init: Box::new(Stmt::Assign { lhs: Expr::Ident(v.clone()), rhs: l, nonblocking: false, timing: None, line }),
                cond,
                step: Box::new(Stmt::Assign { lhs: Expr::Ident(v.clone()), rhs: step, nonblocking: false, timing: None, line }),
                body: Box::new(body),
                line,
            };
            names.push(Declarator { name: v.clone(), dims: vec![], heap: None, init: None, line });
        }
        let d = Decl {
            dir: None,
            kind: NetKind::Int(32),
            net: NetType::Wire,
            explicit_kind: true,
            real_net: false,
            strength: None,
            delay: None,
            signed: true,
            range: None,
            packed: vec![],
            type_param: None,
            ty: None,
            names,
            line,
        };
        Ok(Stmt::Block { name: None, decls: vec![d], stmts: vec![body], fork: None })
    }
}
