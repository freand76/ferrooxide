//! Properties and sequences (IEEE 1800 A.2.10, 16): one precedence-climbing parser for both.
//!
//! From the lowest binding: `always`/`eventually`/`if`/`case`/`accept_on`... (prefix),
//! `|->` `|=>` `#-#` `#=#` (right), `until` & co. and `implies` (right), `iff` (right), `or`,
//! `and`, `not`/`nexttime` (prefix), `intersect`, `within`, `throughout` (right), `##`,
//! repetitions `[*n]` `[=n]` `[->n]` (postfix).

use super::Parser;
use crate::syntax::kw::Kw;
use crate::syntax::token::T;
use crate::syntax::tree::*;
use crate::Result;

fn bp(p: Prop) -> Box<Prop> {
    Box::new(p)
}

impl Parser {
    /// `[@(ev)] [disable iff (e)] property_expr`
    pub(crate) fn property_spec(&mut self) -> Result<Prop> {
        let clock = if self.is_p("@") {
            self.bump();
            Some(self.event_control()?)
        } else {
            None
        };
        let p = if self.is_kw(Kw::Disable) && self.is_kw_at(1, Kw::Iff) {
            self.bump();
            self.bump();
            self.expect_p("(")?;
            let e = self.expr()?;
            self.expect_p(")")?;
            Prop::DisableIff(e, bp(self.prop()?))
        } else {
            self.prop()?
        };
        Ok(match clock {
            Some(c) => Prop::Clocked(c, bp(p)),
            None => p,
        })
    }

    /// A property expression.
    pub(crate) fn prop(&mut self) -> Result<Prop> {
        match self.kw() {
            Some(k @ (Kw::Always | Kw::SAlways | Kw::Eventually | Kw::SEventually)) => {
                self.bump();
                let range = if self.is_p("[") { Some(self.bracket_range()?) } else { None };
                return Ok(Prop::Unary(k, range, bp(self.prop()?)));
            }
            Some(k @ (Kw::AcceptOn | Kw::RejectOn | Kw::SyncAcceptOn | Kw::SyncRejectOn)) => {
                self.bump();
                self.expect_p("(")?;
                let e = self.expr()?;
                self.expect_p(")")?;
                return Ok(Prop::Abort(k, e, bp(self.prop()?)));
            }
            Some(Kw::If) => {
                self.bump();
                self.expect_p("(")?;
                let c = self.expr()?;
                self.expect_p(")")?;
                let t = self.prop()?;
                let e = if self.eat_kw(Kw::Else) { Some(bp(self.prop()?)) } else { None };
                return Ok(Prop::If(c, bp(t), e));
            }
            Some(Kw::Case) => {
                self.bump();
                self.expect_p("(")?;
                let e = self.expr()?;
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
                    let p = self.prop()?;
                    self.expect_p(";")?;
                    items.push((labels, p));
                }
                return Ok(Prop::Case(e, items));
            }
            _ => {}
        }
        if self.is_p("@") {
            self.bump();
            let ec = self.event_control()?;
            return Ok(Prop::Clocked(ec, bp(self.prop()?)));
        }
        let lhs = self.prop_until()?;
        let op = match self.peek() {
            T::Punct("|->") => PropOp::ImplOverlap,
            T::Punct("|=>") => PropOp::ImplNext,
            T::Punct("#-#") => PropOp::FollowOverlap,
            T::Punct("#=#") => PropOp::FollowNext,
            _ => return Ok(lhs),
        };
        self.bump();
        let rhs = self.prop()?;
        Ok(Prop::Binary(op, bp(lhs), bp(rhs)))
    }

    fn prop_until(&mut self) -> Result<Prop> {
        let lhs = self.prop_iff()?;
        let op = match self.kw() {
            Some(Kw::Until) => PropOp::Until,
            Some(Kw::SUntil) => PropOp::SUntil,
            Some(Kw::UntilWith) => PropOp::UntilWith,
            Some(Kw::SUntilWith) => PropOp::SUntilWith,
            Some(Kw::Implies) => PropOp::Implies,
            _ => return Ok(lhs),
        };
        self.bump();
        let rhs = self.prop()?;
        Ok(Prop::Binary(op, bp(lhs), bp(rhs)))
    }

    fn prop_iff(&mut self) -> Result<Prop> {
        let lhs = self.prop_or()?;
        if self.eat_kw(Kw::Iff) {
            let rhs = self.prop()?;
            return Ok(Prop::Binary(PropOp::Iff, bp(lhs), bp(rhs)));
        }
        Ok(lhs)
    }

    fn prop_or(&mut self) -> Result<Prop> {
        let mut lhs = self.prop_and()?;
        while self.eat_kw(Kw::Or) {
            let rhs = self.prop_and()?;
            lhs = Prop::Binary(PropOp::Or, bp(lhs), bp(rhs));
        }
        Ok(lhs)
    }

    fn prop_and(&mut self) -> Result<Prop> {
        let mut lhs = self.prop_not()?;
        while self.eat_kw(Kw::And) {
            let rhs = self.prop_not()?;
            lhs = Prop::Binary(PropOp::And, bp(lhs), bp(rhs));
        }
        Ok(lhs)
    }

    fn prop_not(&mut self) -> Result<Prop> {
        match self.kw() {
            Some(Kw::Not) => {
                self.bump();
                Ok(Prop::Not(bp(self.prop_not()?)))
            }
            Some(k @ (Kw::Nexttime | Kw::SNexttime)) => {
                self.bump();
                let range = if self.is_p("[") { Some(self.bracket_range()?) } else { None };
                Ok(Prop::Unary(k, range, bp(self.prop_not()?)))
            }
            Some(k @ (Kw::Strong | Kw::Weak)) if self.is_p_at(1, "(") => {
                self.bump();
                self.bump();
                let p = self.prop()?;
                self.expect_p(")")?;
                let s = Prop::Strength(k == Kw::Strong, bp(p));
                self.prop_seq_rest(s)
            }
            _ => self.prop_intersect(),
        }
    }

    /// The sequence operators after an operand already parsed.
    fn prop_seq_rest(&mut self, p: Prop) -> Result<Prop> {
        Ok(p)
    }

    fn prop_intersect(&mut self) -> Result<Prop> {
        let mut lhs = self.prop_within()?;
        while self.eat_kw(Kw::Intersect) {
            let rhs = self.prop_within()?;
            lhs = Prop::Binary(PropOp::Intersect, bp(lhs), bp(rhs));
        }
        Ok(lhs)
    }

    fn prop_within(&mut self) -> Result<Prop> {
        let mut lhs = self.prop_throughout()?;
        while self.eat_kw(Kw::Within) {
            let rhs = self.prop_throughout()?;
            lhs = Prop::Binary(PropOp::Within, bp(lhs), bp(rhs));
        }
        Ok(lhs)
    }

    fn prop_throughout(&mut self) -> Result<Prop> {
        let lhs = self.prop_delay()?;
        if self.eat_kw(Kw::Throughout) {
            let rhs = self.prop_throughout()?;
            return Ok(Prop::Binary(PropOp::Throughout, bp(lhs), bp(rhs)));
        }
        Ok(lhs)
    }

    /// `[##d] rep { ##d rep }`
    fn prop_delay(&mut self) -> Result<Prop> {
        let mut lhs = if self.is_p("##") {
            self.bump();
            let d = self.cycle_range()?;
            let r = self.prop_rep()?;
            Prop::Delay(None, d, bp(r))
        } else {
            self.prop_rep()?
        };
        while self.eat_p("##") {
            let d = self.cycle_range()?;
            let r = self.prop_rep()?;
            lhs = Prop::Delay(Some(bp(lhs)), d, bp(r));
        }
        Ok(lhs)
    }

    /// After `##`: `n`, `name`, `(e)`, `[a:b]`, `[*]`, `[+]`
    fn cycle_range(&mut self) -> Result<CycleRange> {
        let loc = self.loc();
        if self.eat_p("[") {
            let r = if self.eat_p("*") {
                CycleRange::Range(Expr::new(ExprKind::Num(crate::parser::dec(0)), loc), Expr::new(ExprKind::Dollar, loc))
            } else if self.eat_p("+") {
                CycleRange::Range(Expr::new(ExprKind::Num(crate::parser::dec(1)), loc), Expr::new(ExprKind::Dollar, loc))
            } else {
                let a = self.expr()?;
                if self.eat_p(":") {
                    CycleRange::Range(a, self.expr()?)
                } else {
                    CycleRange::One(a)
                }
            };
            self.expect_p("]")?;
            return Ok(r);
        }
        if self.eat_p("(") {
            let e = self.expr()?;
            self.expect_p(")")?;
            return Ok(CycleRange::One(e));
        }
        Ok(CycleRange::One(self.delay_primary()?))
    }

    /// `[n]` / `[a:b]` / `[a:$]` after always / nexttime ...
    fn bracket_range(&mut self) -> Result<CycleRange> {
        self.expect_p("[")?;
        let a = self.expr()?;
        let r = if self.eat_p(":") { CycleRange::Range(a, self.expr()?) } else { CycleRange::One(a) };
        self.expect_p("]")?;
        Ok(r)
    }

    fn prop_rep(&mut self) -> Result<Prop> {
        let mut p = self.prop_primary()?;
        while self.is_repetition_bracket() {
            self.bump();
            let kind = match self.bump() {
                T::Punct("*") => RepeatKind::Consecutive,
                T::Punct("=") => RepeatKind::NonConsecutive,
                T::Punct("->") => RepeatKind::Goto,
                T::Punct("+") => RepeatKind::Plus,
                _ => unreachable!(),
            };
            let range = if self.is_p("]") {
                None
            } else {
                let a = self.expr()?;
                Some(if self.eat_p(":") { CycleRange::Range(a, self.expr()?) } else { CycleRange::One(a) })
            };
            self.expect_p("]")?;
            p = Prop::Repeat(bp(p), kind, range);
        }
        Ok(p)
    }

    fn prop_primary(&mut self) -> Result<Prop> {
        if self.is_kw(Kw::FirstMatch) && self.is_p_at(1, "(") {
            self.bump();
            self.bump();
            let p = self.prop()?;
            let mut items = Vec::new();
            while self.eat_p(",") {
                items.push(self.expr_or_assign()?);
            }
            self.expect_p(")")?;
            return Ok(Prop::FirstMatch(bp(p), items));
        }
        if self.is_p("(") {
            // an expression in parentheses that goes on as an expression (`(a + b) == c`), or
            // a parenthesized property
            let save = self.pos;
            if let Some(e) = self.attempt(|p| {
                let e = p.expr()?;
                if p.prop_follows() {
                    Ok(e)
                } else {
                    Err(crate::Error { msg: String::new(), file: String::new(), line: 0 })
                }
            }) {
                return Ok(Prop::Expr(e));
            }
            self.pos = save;
            self.bump();
            let p = self.property_spec()?;
            if self.eat_p(",") {
                let mut items = vec![self.expr_or_assign()?];
                while self.eat_p(",") {
                    items.push(self.expr_or_assign()?);
                }
                self.expect_p(")")?;
                return Ok(Prop::MatchItems(bp(p), items));
            }
            self.expect_p(")")?;
            return Ok(p);
        }
        Ok(Prop::Expr(self.expr()?))
    }

    /// What may follow a complete property operand.
    fn prop_follows(&self) -> bool {
        match self.peek() {
            T::Punct(")" | ";" | "," | "##" | "|->" | "|=>" | "#-#" | "#=#" | "]") => true,
            T::Punct("[") => self.is_repetition_bracket(),
            T::Kw(k) => matches!(
                k,
                Kw::And
                    | Kw::Or
                    | Kw::Iff
                    | Kw::Until
                    | Kw::SUntil
                    | Kw::UntilWith
                    | Kw::SUntilWith
                    | Kw::Implies
                    | Kw::Intersect
                    | Kw::Within
                    | Kw::Throughout
                    | Kw::Else
                    | Kw::Endproperty
                    | Kw::Endsequence
            ),
            T::Eof => true,
            _ => false,
        }
    }

    /// `property name [(ports)]; decls spec; endproperty [: name]`
    pub(crate) fn property_decl(&mut self) -> Result<PropertyDecl> {
        let loc = self.loc();
        self.expect_kw(Kw::Property)?;
        let name = self.ident()?;
        let ports = if self.is_p("(") { self.tf_ports()? } else { vec![] };
        self.expect_p(";")?;
        let mut decls = Vec::new();
        while self.is_block_decl_start() && !self.is_p("@") {
            self.block_decl(&mut decls)?;
        }
        let prop = self.property_spec()?;
        self.eat_p(";");
        self.expect_kw(Kw::Endproperty)?;
        self.end_label()?;
        Ok(PropertyDecl { name, ports, decls, prop, loc })
    }

    /// `sequence name [(ports)]; decls seq; endsequence [: name]`
    pub(crate) fn sequence_decl(&mut self) -> Result<SequenceDecl> {
        let loc = self.loc();
        self.expect_kw(Kw::Sequence)?;
        let name = self.ident()?;
        let ports = if self.is_p("(") { self.tf_ports()? } else { vec![] };
        self.expect_p(";")?;
        let mut decls = Vec::new();
        while self.is_block_decl_start() && !self.is_p("@") {
            self.block_decl(&mut decls)?;
        }
        let seq = self.property_spec()?;
        self.eat_p(";");
        self.expect_kw(Kw::Endsequence)?;
        self.end_label()?;
        Ok(SequenceDecl { name, ports, decls, seq, loc })
    }
}
