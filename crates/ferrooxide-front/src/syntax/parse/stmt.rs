//! Statements (IEEE 1800 A.6).

use super::Parser;
use crate::syntax::kw::Kw;
use crate::syntax::token::T;
use crate::syntax::tree::*;
use crate::Result;

fn st(kind: StmtKind, loc: Loc) -> Stmt {
    Stmt { kind, label: None, loc }
}

impl Parser {
    /// A soft keyword at the cursor used as a name (`bit = 1;`, `x = int + 1`, `wire logic;`):
    /// what follows can only follow a name.
    pub(crate) fn soft_name_here(&self) -> bool {
        let T::Kw(k) = self.peek() else { return false };
        if k.is_hard() {
            return false;
        }
        match k {
            // `return -x;`: an operator after return; only an assignment makes it a name
            Kw::Return => return matches!(self.peek_at(1), T::Punct("=" | "<=")),
            Kw::Break | Kw::Continue if self.is_p_at(1, ";") => return false,
            // `export *::*;`
            Kw::Export if self.is_p_at(1, "*") => return false,
            Kw::This | Kw::Super | Kw::Null | Kw::New | Kw::Local => return false,
            _ => {}
        }
        matches!(
            self.peek_at(1),
            T::Punct("=" | "<=" | ")" | "," | ";" | "." | "+" | "-" | "*" | "/" | "&" | "|" | "^" | "==" | "!=" | "?" | ":" | "}" | "]")
        )
    }

    /// A keyword IEEE 1800 added that starts a statement, used as a name instead
    /// (`expect[3] = 1;`): what follows does not fit the statement.
    fn soft_stmt_kw_as_name(&self) -> bool {
        let Some(k) = self.kw() else { return false };
        match k {
            Kw::Expect | Kw::WaitOrder | Kw::Randsequence | Kw::Foreach => !self.is_p_at(1, "("),
            Kw::Assert | Kw::Assume | Kw::Cover => {
                !(self.is_p_at(1, "(") || self.is_p_at(1, "#") || matches!(self.peek_at(1), T::Kw(Kw::Property | Kw::Sequence | Kw::Final)))
            }
            Kw::Restrict => !self.is_kw_at(1, Kw::Property),
            Kw::Let => !self.is_ident_at(1),
            _ => false,
        }
    }

    /// A statement (or `;`).
    pub(crate) fn stmt(&mut self) -> Result<Stmt> {
        let loc = self.loc();
        // `label: statement`
        if self.is_ident() && self.is_p_at(1, ":") && !self.soft_name_here_kwlike() {
            let label = self.ident()?;
            self.bump();
            if self.is_kw(Kw::Begin) || self.is_kw(Kw::Fork) {
                self.block_label = Some(label.clone());
            }
            let mut s = self.stmt()?;
            s.label = Some(label);
            return Ok(s);
        }
        if self.soft_name_here() || self.soft_stmt_kw_as_name() {
            return self.simple_stmt();
        }
        let kind = match self.peek().clone() {
            T::Punct(";") => {
                self.bump();
                StmtKind::Null
            }
            T::Punct("#") => {
                self.bump();
                let d = self.delay_value()?;
                let body = self.stmt()?;
                StmtKind::Timed { timing: Timing::Delay(d), body: Box::new(body) }
            }
            T::Punct("##") => {
                self.bump();
                let c = self.cycle_delay()?;
                let body = self.stmt()?;
                StmtKind::Timed { timing: Timing::Cycle(c), body: Box::new(body) }
            }
            T::Punct("@") => {
                self.bump();
                let ec = self.event_control()?;
                let body = self.stmt()?;
                StmtKind::Timed { timing: Timing::Event(ec), body: Box::new(body) }
            }
            T::Punct("->") | T::Punct("->>") => {
                let nonblocking = self.is_p("->>");
                self.bump();
                let timing = if nonblocking { self.opt_timing()? } else { None };
                let target = self.postfix_expr()?;
                self.expect_p(";")?;
                StmtKind::Trigger { nonblocking, timing, target }
            }
            T::Kw(Kw::Begin) | T::Kw(Kw::Fork) => self.block()?,
            T::Kw(Kw::If) => self.if_stmt(None)?,
            T::Kw(k @ (Kw::Unique | Kw::Unique0 | Kw::Priority)) => {
                self.bump();
                if self.is_kw(Kw::If) {
                    self.if_stmt(Some(k))?
                } else if matches!(self.kw(), Some(Kw::Case | Kw::Casez | Kw::Casex)) {
                    self.case_stmt(Some(k))?
                } else {
                    return self.err(format!("expected 'if' or 'case' after '{}'", k.as_str()));
                }
            }
            T::Kw(Kw::Case | Kw::Casez | Kw::Casex) => self.case_stmt(None)?,
            T::Kw(Kw::Randcase) => {
                self.bump();
                let mut items = Vec::new();
                while !self.eat_kw(Kw::Endcase) {
                    let w = self.expr()?;
                    self.expect_p(":")?;
                    items.push((w, self.stmt()?));
                }
                StmtKind::RandCase(items)
            }
            T::Kw(Kw::For) => self.for_stmt()?,
            T::Kw(Kw::Foreach) => {
                self.bump();
                let (array, vars) = self.foreach_head()?;
                let body = self.stmt()?;
                StmtKind::Foreach { array, vars, body: Box::new(body) }
            }
            T::Kw(Kw::While) => {
                self.bump();
                self.expect_p("(")?;
                let cond = self.expr()?;
                self.expect_p(")")?;
                StmtKind::While { cond, body: Box::new(self.stmt()?) }
            }
            T::Kw(Kw::Do) => {
                self.bump();
                let body = self.stmt()?;
                self.expect_kw(Kw::While)?;
                self.expect_p("(")?;
                let cond = self.expr()?;
                self.expect_p(")")?;
                self.expect_p(";")?;
                StmtKind::DoWhile { body: Box::new(body), cond }
            }
            T::Kw(Kw::Repeat) => {
                self.bump();
                self.expect_p("(")?;
                let count = self.expr()?;
                self.expect_p(")")?;
                StmtKind::Repeat { count, body: Box::new(self.stmt()?) }
            }
            T::Kw(Kw::Forever) => {
                self.bump();
                StmtKind::Forever(Box::new(self.stmt()?))
            }
            T::Kw(Kw::Wait) => {
                self.bump();
                if self.eat_kw(Kw::Fork) {
                    self.expect_p(";")?;
                    StmtKind::WaitFork
                } else {
                    self.expect_p("(")?;
                    let cond = self.expr()?;
                    self.expect_p(")")?;
                    StmtKind::Wait { cond, body: Box::new(self.stmt()?) }
                }
            }
            T::Kw(Kw::WaitOrder) => {
                self.bump();
                self.expect_p("(")?;
                let mut events = Vec::new();
                loop {
                    events.push(self.postfix_expr()?);
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(")")?;
                let action = self.action_block()?;
                StmtKind::WaitOrder { events, action: Box::new(action) }
            }
            T::Kw(Kw::Disable) => {
                self.bump();
                if self.eat_kw(Kw::Fork) {
                    self.expect_p(";")?;
                    StmtKind::DisableFork
                } else {
                    let e = self.postfix_expr()?;
                    self.expect_p(";")?;
                    StmtKind::Disable(e)
                }
            }
            T::Kw(k @ (Kw::Force | Kw::Release | Kw::Assign | Kw::Deassign)) => {
                self.bump();
                let lhs = self.postfix_expr()?;
                let rhs = if matches!(k, Kw::Force | Kw::Assign) {
                    self.expect_p("=")?;
                    Some(self.expr()?)
                } else {
                    None
                };
                self.expect_p(";")?;
                StmtKind::ProcCont { kw: k, lhs, rhs }
            }
            T::Kw(Kw::Return) => {
                self.bump();
                let v = if self.is_p(";") { None } else { Some(self.expr()?) };
                self.expect_p(";")?;
                StmtKind::Return(v)
            }
            T::Kw(Kw::Break) => {
                self.bump();
                self.expect_p(";")?;
                StmtKind::Break
            }
            T::Kw(Kw::Continue) => {
                self.bump();
                self.expect_p(";")?;
                StmtKind::Continue
            }
            T::Kw(Kw::Assert | Kw::Assume | Kw::Cover | Kw::Restrict) => {
                let a = self.assertion(None)?;
                StmtKind::Assertion(Box::new(a))
            }
            T::Kw(Kw::Expect) => {
                self.bump();
                self.expect_p("(")?;
                let prop = self.property_spec()?;
                self.expect_p(")")?;
                let action = self.action_block()?;
                StmtKind::Expect { prop, action: Box::new(action) }
            }
            T::Kw(Kw::Randsequence) => self.randsequence()?,
            T::Kw(Kw::Let) => {
                let it = self.let_decl()?;
                StmtKind::Item(Box::new(it))
            }
            T::Kw(k) if k.is_hard() && !matches!(k, Kw::Assign) => {
                return self.err(format!("unexpected '{}' in statement", k.as_str()));
            }
            _ => return self.simple_stmt(),
        };
        Ok(st(kind, loc))
    }

    /// `name:` where `name` could also start a statement-like construct (never: a label is
    /// always a plain name); kept for clarity of the label rule.
    fn soft_name_here_kwlike(&self) -> bool {
        false
    }

    /// An assignment, an increment, or a subroutine call, with the `;`.
    pub(crate) fn simple_stmt(&mut self) -> Result<Stmt> {
        let s = self.assign_like()?;
        self.expect_p(";")?;
        Ok(s)
    }

    /// An assignment / increment / call without the terminator (also `for` steps).
    pub(crate) fn assign_like(&mut self) -> Result<Stmt> {
        let loc = self.loc();
        if self.is_p("++") || self.is_p("--") {
            let inc = self.is_p("++");
            self.bump();
            let e = self.postfix_expr()?;
            return Ok(st(StmtKind::Expr(Expr::new(ExprKind::IncDec { prefix: true, inc, e: Box::new(e) }, loc)), loc));
        }
        let lhs = self.postfix_expr()?;
        if self.is_p("++") || self.is_p("--") {
            let inc = self.is_p("++");
            self.bump();
            return Ok(st(StmtKind::Expr(Expr::new(ExprKind::IncDec { prefix: false, inc, e: Box::new(lhs) }, loc)), loc));
        }
        if self.eat_p("<=") {
            let timing = self.opt_timing()?;
            let rhs = self.expr()?;
            return Ok(st(StmtKind::NonBlocking { lhs, timing, rhs }, loc));
        }
        if let Some(op) = self.assign_op() {
            self.bump();
            let timing = if op == AssignOp::Eq { self.opt_timing()? } else { None };
            let rhs = self.expr()?;
            return Ok(st(StmtKind::Assign { lhs, op, timing, rhs }, loc));
        }
        if self.is_p(";") || self.is_p(",") || self.is_p(")") {
            return Ok(st(StmtKind::Expr(lhs), loc));
        }
        self.err(format!("expected '=' or '<=', found {}", self.describe()))
    }

    /// An intra-assignment timing control: `#d`, `@e`, `repeat (n) @e`, `##n`.
    pub(crate) fn opt_timing(&mut self) -> Result<Option<Timing>> {
        Ok(if self.eat_p("#") {
            Some(Timing::Delay(self.delay_value()?))
        } else if self.eat_p("##") {
            Some(Timing::Cycle(self.cycle_delay()?))
        } else if self.eat_p("@") {
            Some(Timing::Event(self.event_control()?))
        } else if self.is_kw(Kw::Repeat) {
            self.bump();
            self.expect_p("(")?;
            let n = self.expr()?;
            self.expect_p(")")?;
            self.expect_p("@")?;
            Some(Timing::Repeat(n, self.event_control()?))
        } else {
            None
        })
    }

    /// After `#`: `5`, `d`, `1ns`, `(e)`, `(rise, fall, off)` (each may be min:typ:max).
    pub(crate) fn delay_value(&mut self) -> Result<Delay> {
        if self.eat_p("(") {
            let mut v = vec![self.mintypmax()?];
            while self.eat_p(",") {
                v.push(self.mintypmax()?);
            }
            self.expect_p(")")?;
            return Ok(if v.len() == 1 { Delay::Value(v.pop().unwrap()) } else { Delay::List(v) });
        }
        Ok(Delay::Value(self.delay_primary()?))
    }

    /// `e` or `min:typ:max`
    pub(crate) fn mintypmax(&mut self) -> Result<Expr> {
        let loc = self.loc();
        let e = self.expr()?;
        if self.eat_p(":") {
            let t = self.expr()?;
            self.expect_p(":")?;
            let m = self.expr()?;
            return Ok(Expr::new(ExprKind::MinTypMax(Box::new(e), Box::new(t), Box::new(m)), loc));
        }
        Ok(e)
    }

    /// After `##`: `n`, `name`, `(e)`, `[a:b]`, `[*]`, `[+]`
    pub(crate) fn cycle_delay(&mut self) -> Result<Expr> {
        let loc = self.loc();
        if self.eat_p("[") {
            let r = if self.eat_p("*") {
                let d = Expr::new(ExprKind::Dollar, loc);
                Expr::new(ExprKind::Range(Box::new(Expr::new(ExprKind::Null, loc)), RangeKind::Colon, Box::new(Expr::new(ExprKind::Num(crate::ast::dec(0)), loc)), Box::new(d)), loc)
            } else if self.eat_p("+") {
                let d = Expr::new(ExprKind::Dollar, loc);
                Expr::new(ExprKind::Range(Box::new(Expr::new(ExprKind::Null, loc)), RangeKind::Colon, Box::new(Expr::new(ExprKind::Num(crate::ast::dec(1)), loc)), Box::new(d)), loc)
            } else {
                let a = self.expr()?;
                self.expect_p(":")?;
                let b = self.expr()?;
                Expr::new(ExprKind::Range(Box::new(Expr::new(ExprKind::Null, loc)), RangeKind::Colon, Box::new(a), Box::new(b)), loc)
            };
            self.expect_p("]")?;
            return Ok(r);
        }
        if self.eat_p("(") {
            let e = self.expr()?;
            self.expect_p(")")?;
            return Ok(e);
        }
        self.delay_primary()
    }

    /// After `@`: `*`, `(*)`, `(event list)`, `name`, `a.b`.
    pub(crate) fn event_control(&mut self) -> Result<EventCtl> {
        if self.eat_p("*") {
            return Ok(EventCtl::Star);
        }
        if !self.is_p("(") {
            let loc = self.loc();
            let e = self.postfix_expr()?;
            return Ok(EventCtl::List(vec![EventExpr { edge: None, expr: e, iff: None, loc }]));
        }
        self.bump();
        if self.eat_p("*") {
            self.expect_p(")")?;
            return Ok(EventCtl::Star);
        }
        let mut v = Vec::new();
        loop {
            let loc = self.loc();
            let edge = self.edge();
            let expr = self.expr()?;
            let iff = if self.eat_kw(Kw::Iff) { Some(self.expr()?) } else { None };
            v.push(EventExpr { edge, expr, iff, loc });
            if !(self.eat_kw(Kw::Or) || self.eat_p(",")) {
                break;
            }
        }
        self.expect_p(")")?;
        Ok(EventCtl::List(v))
    }

    pub(crate) fn edge(&mut self) -> Option<Edge> {
        if self.eat_kw(Kw::Posedge) {
            Some(Edge::Pos)
        } else if self.eat_kw(Kw::Negedge) {
            Some(Edge::Neg)
        } else if self.eat_kw(Kw::Edge) {
            Some(Edge::Both)
        } else {
            None
        }
    }

    /// `begin`/`fork` block (the cursor at the keyword).
    fn block(&mut self) -> Result<StmtKind> {
        let fork = self.is_kw(Kw::Fork);
        let label = self.block_label.take();
        self.bump();
        let name = self.end_label()?;
        self.push_scope();
        let mut decls = Vec::new();
        let mut stmts = Vec::new();
        let join = loop {
            if !fork && self.eat_kw(Kw::End) {
                break None;
            }
            if fork {
                if self.eat_kw(Kw::Join) {
                    break Some(Join::All);
                }
                if self.eat_kw(Kw::JoinAny) {
                    break Some(Join::Any);
                }
                if self.eat_kw(Kw::JoinNone) {
                    break Some(Join::None);
                }
            }
            if self.at_eof() {
                return self.err("unexpected end of file in block");
            }
            if self.is_block_decl_start() {
                self.block_decl(&mut decls)?;
            } else {
                stmts.push(self.stmt()?);
            }
        };
        self.pop_scope();
        self.close_label(name.as_deref().or(label.as_deref()))?;
        Ok(StmtKind::Block { fork: if fork { join } else { None }, name, decls, stmts })
    }

    fn if_stmt(&mut self, qual: Qualifier) -> Result<StmtKind> {
        self.expect_kw(Kw::If)?;
        self.expect_p("(")?;
        let cond = self.cond_predicate()?;
        self.expect_p(")")?;
        let then_ = Box::new(self.stmt()?);
        let else_ = if self.eat_kw(Kw::Else) { Some(Box::new(self.stmt()?)) } else { None };
        Ok(StmtKind::If { qual, cond, then_, else_ })
    }

    fn case_stmt(&mut self, qual: Qualifier) -> Result<StmtKind> {
        let kw = match self.bump() {
            T::Kw(Kw::Casez) => CaseKw::Casez,
            T::Kw(Kw::Casex) => CaseKw::Casex,
            _ => CaseKw::Case,
        };
        self.expect_p("(")?;
        let expr = self.expr()?;
        self.expect_p(")")?;
        let mode = if self.eat_kw(Kw::Inside) {
            CaseMode::Inside
        } else if self.eat_kw(Kw::Matches) {
            CaseMode::Matches
        } else {
            CaseMode::Normal
        };
        let mut items = Vec::new();
        while !self.eat_kw(Kw::Endcase) {
            if self.at_eof() {
                return self.err("unexpected end of file in case");
            }
            let loc = self.loc();
            if self.eat_kw(Kw::Default) {
                self.eat_p(":");
                let body = self.stmt()?;
                items.push(CaseItem { labels: vec![], pattern: None, guard: None, body, loc });
                continue;
            }
            let mut labels = Vec::new();
            let mut pattern = None;
            let mut guard = None;
            if mode == CaseMode::Matches {
                pattern = Some(self.pattern()?);
                if self.is_triple_and() {
                    self.bump();
                    self.bump();
                    guard = Some(self.expr()?);
                }
            } else {
                loop {
                    labels.push(if mode == CaseMode::Inside { self.value_range()? } else { ValueRange::Value(self.expr()?) });
                    if !self.eat_p(",") {
                        break;
                    }
                }
            }
            self.expect_p(":")?;
            let body = self.stmt()?;
            items.push(CaseItem { labels, pattern, guard, body, loc });
        }
        Ok(StmtKind::Case { qual, kw, expr, mode, items })
    }

    fn for_stmt(&mut self) -> Result<StmtKind> {
        self.expect_kw(Kw::For)?;
        self.expect_p("(")?;
        self.push_scope();
        let mut init = Vec::new();
        let mut decl_seen = false;
        if !self.is_p(";") {
            loop {
                let loc = self.loc();
                let var = self.eat_kw(Kw::Var);
                let typed = var || (!self.soft_name_here() && self.is_type_start() && !self.is_p_at(1, "="));
                if typed || decl_seen {
                    let ty = if typed { Some(self.data_type_or_implicit()?) } else { None };
                    let name = self.ident()?;
                    self.expect_p("=")?;
                    let e = self.expr()?;
                    init.push(ForInit::Decl { var, ty, name, init: e, loc });
                    decl_seen = true;
                } else {
                    init.push(ForInit::Assign(self.assign_like()?));
                }
                if !self.eat_p(",") {
                    break;
                }
            }
        }
        self.expect_p(";")?;
        let cond = if self.is_p(";") { None } else { Some(self.expr()?) };
        self.expect_p(";")?;
        let mut step = Vec::new();
        if !self.is_p(")") {
            loop {
                step.push(self.assign_like()?);
                if !self.eat_p(",") {
                    break;
                }
            }
        }
        self.expect_p(")")?;
        let body = Box::new(self.stmt()?);
        self.pop_scope();
        Ok(StmtKind::For { init, cond, step, body })
    }

    /// `( array [ i, , k ] )` of foreach (also `[i][j]`).
    pub(crate) fn foreach_head(&mut self) -> Result<(Expr, Vec<Option<String>>)> {
        self.expect_p("(")?;
        let loc = self.loc();
        let mut array = if self.is_kw(Kw::This) {
            self.bump();
            Expr::new(ExprKind::This, loc)
        } else {
            Expr::new(ExprKind::Ident(self.ident()?), loc)
        };
        loop {
            // `a[i].m[j]`: selects before a member belong to the array's name
            if self.is_p("[") && self.after_brackets(0).is_some_and(|k| self.is_p_at(k, ".")) {
                self.bump();
                let i = self.expr()?;
                self.expect_p("]")?;
                array = Expr::new(ExprKind::Index(Box::new(array), Box::new(i)), loc);
                continue;
            }
            if !self.eat_p(".") {
                break;
            }
            let m = self.member_name()?;
            array = Expr::new(ExprKind::Member(Box::new(array), m), loc);
        }
        if self.is_p("::") {
            return self.err("foreach over a scoped name is not supported");
        }
        let mut vars = Vec::new();
        while self.eat_p("[") {
            loop {
                vars.push(if self.is_ident() { Some(self.ident()?) } else { None });
                if !self.eat_p(",") {
                    break;
                }
            }
            self.expect_p("]")?;
        }
        self.expect_p(")")?;
        Ok((array, vars))
    }

    /// `pass_stmt [else fail_stmt]` / `else fail_stmt` / `;` after an assertion.
    pub(crate) fn action_block(&mut self) -> Result<ActionBlock> {
        if self.eat_kw(Kw::Else) {
            return Ok(ActionBlock { pass: None, fail: Some(self.stmt()?) });
        }
        let pass = self.stmt()?;
        let pass = if matches!(pass.kind, StmtKind::Null) { None } else { Some(pass) };
        let fail = if self.eat_kw(Kw::Else) { Some(self.stmt()?) } else { None };
        Ok(ActionBlock { pass, fail })
    }

    /// An immediate, deferred or concurrent assertion (statement or module item), the cursor
    /// at assert/assume/cover/restrict.
    pub(crate) fn assertion(&mut self, label: Option<String>) -> Result<Assertion> {
        let loc = self.loc();
        let kind = match self.bump() {
            T::Kw(Kw::Assert) => AssertKind::Assert,
            T::Kw(Kw::Assume) => AssertKind::Assume,
            T::Kw(Kw::Cover) => AssertKind::Cover,
            _ => AssertKind::Restrict,
        };
        let body = if self.eat_kw(Kw::Property) {
            self.expect_p("(")?;
            let p = self.property_spec()?;
            self.expect_p(")")?;
            AssertBody::Property(p)
        } else if self.eat_kw(Kw::Sequence) {
            self.expect_p("(")?;
            let p = self.property_spec()?;
            self.expect_p(")")?;
            AssertBody::Sequence(p)
        } else {
            let deferred = if self.is_p("#") && matches!(self.peek_at(1), T::Num(n) if n.digits == "0") {
                self.bump();
                self.bump();
                Some(Deferral::Zero)
            } else if self.eat_kw(Kw::Final) {
                Some(Deferral::Final)
            } else {
                None
            };
            self.expect_p("(")?;
            let expr = self.expr()?;
            self.expect_p(")")?;
            AssertBody::Immediate { deferred, expr }
        };
        let action = if matches!(kind, AssertKind::Restrict) {
            self.expect_p(";")?;
            ActionBlock { pass: None, fail: None }
        } else {
            self.action_block()?
        };
        Ok(Assertion { label, kind, body, action, loc })
    }

    fn randsequence(&mut self) -> Result<StmtKind> {
        self.expect_kw(Kw::Randsequence)?;
        self.expect_p("(")?;
        let start = if self.is_ident() { Some(self.ident()?) } else { None };
        self.expect_p(")")?;
        let mut prods = Vec::new();
        while !self.eat_kw(Kw::Endsequence) {
            if self.at_eof() {
                return self.err("unexpected end of file in randsequence");
            }
            let loc = self.loc();
            let ret = if self.is_ident() && (self.is_p_at(1, ":") || self.is_p_at(1, "(")) {
                None
            } else {
                self.data_type()?
            };
            let name = self.ident()?;
            let ports = if self.is_p("(") { Some(self.tf_ports()?) } else { None };
            self.expect_p(":")?;
            let mut rules = Vec::new();
            loop {
                let mut items = Vec::new();
                if self.is_kw(Kw::Rand) && self.is_kw_at(1, Kw::Join) {
                    self.bump();
                    self.bump();
                    let weight = if self.eat_p("(") {
                        let e = self.expr()?;
                        self.expect_p(")")?;
                        Some(e)
                    } else {
                        None
                    };
                    let mut v = Vec::new();
                    while !self.is_p("|") && !self.is_p(";") && !self.is_p(":=") {
                        v.push(self.prod_item()?);
                    }
                    items.push(ProdItem::RandJoin { weight, items: v });
                }
                while !self.is_p("|") && !self.is_p(";") && !self.is_p(":=") {
                    items.push(self.prod_item()?);
                }
                let weight = if self.eat_p(":=") {
                    let w = self.weight_expr()?;
                    if self.is_p("{") {
                        items.push(self.prod_item()?);
                    }
                    Some(w)
                } else {
                    None
                };
                rules.push((items, weight));
                if !self.eat_p("|") {
                    break;
                }
            }
            self.expect_p(";")?;
            prods.push(Production { ret, name, ports, rules, loc });
        }
        Ok(StmtKind::RandSequence { start, prods })
    }

    /// A weight: a number, a name, or `(expr)`.
    fn weight_expr(&mut self) -> Result<Expr> {
        if self.eat_p("(") {
            let e = self.expr()?;
            self.expect_p(")")?;
            return Ok(e);
        }
        self.delay_primary()
    }

    fn prod_item(&mut self) -> Result<ProdItem> {
        if self.eat_p("{") {
            let mut decls = Vec::new();
            let mut stmts = Vec::new();
            while !self.eat_p("}") {
                if self.at_eof() {
                    return self.err("unexpected end of file in a production code block");
                }
                if self.is_block_decl_start() {
                    self.block_decl(&mut decls)?;
                } else {
                    stmts.push(self.stmt()?);
                }
            }
            return Ok(ProdItem::Code(decls, stmts));
        }
        if self.eat_kw(Kw::If) {
            self.expect_p("(")?;
            let cond = self.expr()?;
            self.expect_p(")")?;
            let then_ = Box::new(self.prod_item()?);
            let else_ = if self.eat_kw(Kw::Else) { Some(Box::new(self.prod_item()?)) } else { None };
            return Ok(ProdItem::If { cond, then_, else_ });
        }
        if self.eat_kw(Kw::Repeat) {
            self.expect_p("(")?;
            let count = self.expr()?;
            self.expect_p(")")?;
            return Ok(ProdItem::Repeat { count, item: Box::new(self.prod_item()?) });
        }
        if self.eat_kw(Kw::Case) {
            self.expect_p("(")?;
            let expr = self.expr()?;
            self.expect_p(")")?;
            let mut items = Vec::new();
            while !self.eat_kw(Kw::Endcase) {
                let labels = if self.eat_kw(Kw::Default) {
                    self.eat_p(":");
                    vec![]
                } else {
                    let mut l = vec![self.expr()?];
                    while self.eat_p(",") {
                        l.push(self.expr()?);
                    }
                    self.expect_p(":")?;
                    l
                };
                let it = self.prod_item()?;
                self.expect_p(";")?;
                items.push((labels, it));
            }
            return Ok(ProdItem::Case { expr, items });
        }
        let name = self.ident()?;
        let args = if self.is_p("(") { self.args()? } else { vec![] };
        Ok(ProdItem::Call(name, args))
    }
}
