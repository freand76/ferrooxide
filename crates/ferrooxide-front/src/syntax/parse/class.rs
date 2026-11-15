//! Classes, constraints and covergroups (IEEE 1800 A.1.9, A.1.10, A.2.11).

use super::Parser;
use crate::syntax::kw::Kw;
use crate::syntax::token::T;
use crate::syntax::tree::*;
use crate::Result;

impl Parser {
    /// `[virtual|interface] class [lifetime] name [#(params)] [extends ...] [implements ...];
    /// items endclass` (the name is a type name from here on).
    pub(crate) fn class_decl(&mut self) -> Result<ClassDecl> {
        let loc = self.loc();
        let virtual_ = self.eat_kw(Kw::Virtual);
        let interface = self.eat_kw(Kw::Interface);
        self.expect_kw(Kw::Class)?;
        let lifetime = self.lifetime();
        let name = self.ident()?;
        self.add_type(&name);
        self.push_scope();
        let params = if self.is_p("#") { Some(self.param_port_list()?) } else { None };
        let mut implements = Vec::new();
        let extends = if self.eat_kw(Kw::Extends) {
            let base = self.type_name()?;
            self.inherit_types(&base.name);
            let args = if self.is_p("(") { Some(self.args()?) } else { None };
            // an interface class extends several
            while interface && self.eat_p(",") {
                let b = self.type_name()?;
                self.inherit_types(&b.name);
                implements.push(b);
            }
            Some((base, args))
        } else {
            None
        };
        if self.eat_kw(Kw::Implements) {
            loop {
                implements.push(self.type_name()?);
                if !self.eat_p(",") {
                    break;
                }
            }
        }
        self.expect_p(";")?;
        let mut items = Vec::new();
        while !self.eat_kw(Kw::Endclass) {
            if self.at_eof() {
                return self.err("unexpected end of file in class");
            }
            self.class_item(&mut items)?;
        }
        self.end_label()?;
        let types = self.pop_scope();
        self.classes.insert(name.clone(), types);
        Ok(ClassDecl { virtual_, interface, lifetime, name, params, extends, implements, items, loc })
    }

    /// The type names of a base class become visible in the derived one.
    fn inherit_types(&mut self, base: &str) {
        if let Some(t) = self.classes.get(base).cloned() {
            for n in t {
                self.add_type(&n);
            }
        }
    }

    fn class_item(&mut self, out: &mut Vec<ClassItem>) -> Result<()> {
        // qualifiers
        let mut quals = Vec::new();
        while let Some(k) = self.kw() {
            if matches!(k, Kw::Rand | Kw::Randc | Kw::Static | Kw::Local | Kw::Protected | Kw::Virtual | Kw::Pure | Kw::Extern | Kw::Const | Kw::Automatic)
                && !(k == Kw::Virtual && self.is_ident_at(1) && !self.is_kw_at(1, Kw::Interface) && false)
            {
                // `virtual bus_if vif;` is a property with a virtual interface type
                if k == Kw::Virtual && !matches!(self.peek_at(1), T::Kw(Kw::Function | Kw::Task | Kw::Pure | Kw::Protected | Kw::Local | Kw::Static | Kw::Extern)) {
                    break;
                }
                // `const` before a type is part of the property
                quals.push(k);
                self.bump();
            } else {
                break;
            }
        }
        match self.peek().clone() {
            T::Punct(";") => {
                self.bump();
            }
            T::Kw(Kw::Function | Kw::Task) => out.push(ClassItem::Method(Box::new(self.subroutine(quals)?))),
            T::Kw(Kw::Constraint) => out.push(ClassItem::Constraint(Box::new(self.constraint_decl(quals)?))),
            T::Kw(Kw::Class) | T::Kw(Kw::Interface) if self.is_kw(Kw::Class) || self.is_kw_at(1, Kw::Class) => {
                out.push(ClassItem::Item(Item::Class(Box::new(self.class_decl()?))))
            }
            T::Kw(Kw::Typedef | Kw::Parameter | Kw::Localparam | Kw::Import | Kw::Covergroup | Kw::Let) => {
                let mut v = Vec::new();
                self.module_item(&mut v)?;
                out.extend(v.into_iter().map(ClassItem::Item));
            }
            _ => {
                let mut v = Vec::new();
                self.block_decl(&mut v)?;
                for it in v {
                    out.push(ClassItem::Property { quals: quals.clone(), decl: it });
                }
            }
        }
        Ok(())
    }

    /// `[static] constraint [C::]name { ... }` or a prototype `constraint name;`
    pub(crate) fn constraint_decl(&mut self, mut quals: Vec<Kw>) -> Result<ConstraintDecl> {
        let loc = self.loc();
        let static_ = self.eat_kw(Kw::Static) || quals.contains(&Kw::Static);
        quals.retain(|k| *k != Kw::Static);
        self.expect_kw(Kw::Constraint)?;
        let lloc = self.loc();
        let mut scopes = Vec::new();
        let mut name = self.ident()?;
        while self.eat_p("::") {
            scopes.push(NamePart { name, params: None });
            name = self.ident()?;
        }
        let name = Name { scopes, name, params: None, loc: lloc };
        if self.eat_p(";") {
            return Ok(ConstraintDecl { static_, quals, name, items: None, loc });
        }
        let items = self.constraint_block()?;
        Ok(ConstraintDecl { static_, quals, name, items: Some(items), loc })
    }

    /// `{ constraint items }`
    pub(crate) fn constraint_block(&mut self) -> Result<Vec<ConstraintItem>> {
        self.expect_p("{")?;
        let saved = std::mem::replace(&mut self.in_constraint, true);
        let mut v = Vec::new();
        let r = (|| {
            while !self.eat_p("}") {
                if self.at_eof() {
                    return self.err("unexpected end of file in constraint");
                }
                v.push(self.constraint_item()?);
            }
            Ok(())
        })();
        self.in_constraint = saved;
        r?;
        Ok(v)
    }

    /// A constraint set: one item or a `{ ... }` block.
    fn constraint_set(&mut self) -> Result<Vec<ConstraintItem>> {
        if self.is_p("{") {
            return self.constraint_block();
        }
        Ok(vec![self.constraint_item()?])
    }

    fn constraint_item(&mut self) -> Result<ConstraintItem> {
        match self.kw() {
            Some(Kw::If) => {
                self.bump();
                self.expect_p("(")?;
                let c = self.expr()?;
                self.expect_p(")")?;
                let t = self.constraint_set()?;
                let e = if self.eat_kw(Kw::Else) { Some(self.constraint_set()?) } else { None };
                return Ok(ConstraintItem::If(c, t, e));
            }
            Some(Kw::Foreach) => {
                self.bump();
                let (array, vars) = self.foreach_head()?;
                let body = self.constraint_set()?;
                return Ok(ConstraintItem::Foreach(array, vars, body));
            }
            Some(Kw::Solve) => {
                self.bump();
                let mut a = vec![self.postfix_expr()?];
                while self.eat_p(",") {
                    a.push(self.postfix_expr()?);
                }
                self.expect_kw(Kw::Before)?;
                let mut b = vec![self.postfix_expr()?];
                while self.eat_p(",") {
                    b.push(self.postfix_expr()?);
                }
                self.expect_p(";")?;
                return Ok(ConstraintItem::Solve(a, b));
            }
            Some(Kw::Unique) => {
                self.bump();
                let set = self.range_set()?;
                self.expect_p(";")?;
                return Ok(ConstraintItem::Unique(set));
            }
            Some(Kw::Disable) if self.is_kw_at(1, Kw::Soft) => {
                self.bump();
                self.bump();
                let e = self.postfix_expr()?;
                self.expect_p(";")?;
                return Ok(ConstraintItem::DisableSoft(e));
            }
            Some(Kw::Soft) => {
                self.bump();
                let e = self.expr()?;
                self.expect_p(";")?;
                return Ok(ConstraintItem::Expr { soft: true, expr: e });
            }
            _ => {}
        }
        if self.is_p("{") {
            return Ok(ConstraintItem::Block(self.constraint_block()?));
        }
        let e = self.expr()?;
        if self.eat_p("->") {
            let set = self.constraint_set()?;
            return Ok(ConstraintItem::Implies(e, set));
        }
        self.expect_p(";")?;
        Ok(ConstraintItem::Expr { soft: false, expr: e })
    }

    /// `covergroup name [(ports)] [event]; items endgroup`
    pub(crate) fn covergroup(&mut self) -> Result<Covergroup> {
        let loc = self.loc();
        self.expect_kw(Kw::Covergroup)?;
        let extends = if self.eat_kw(Kw::Extends) { Some(self.ident()?) } else { None };
        let name = self.ident()?;
        self.add_type(&name);
        let ports = if self.is_p("(") { self.tf_ports()? } else { vec![] };
        let event = if self.eat_p("@") {
            Some(CoverEvent::Event(self.event_control()?))
        } else if self.eat_p("@@") {
            self.expect_p("(")?;
            let e = self.block_event()?;
            self.expect_p(")")?;
            Some(CoverEvent::Block(e))
        } else if self.is_kw(Kw::With) {
            self.bump();
            self.expect_kw(Kw::Function)?;
            let n = self.ident()?;
            if n != "sample" {
                return self.err("expected 'sample' after 'with function'");
            }
            Some(CoverEvent::Sample(self.tf_ports()?))
        } else {
            None
        };
        self.expect_p(";")?;
        let mut items = Vec::new();
        while !self.eat_kw(Kw::Endgroup) {
            if self.at_eof() {
                return self.err("unexpected end of file in covergroup");
            }
            if self.eat_p(";") {
                continue;
            }
            items.push(self.cover_item()?);
        }
        self.end_label()?;
        Ok(Covergroup { name, ports, event, items, extends, loc })
    }

    /// `begin f or end g` of `@@(...)`, kept as an expression list
    fn block_event(&mut self) -> Result<Expr> {
        let loc = self.loc();
        let mut parts = Vec::new();
        loop {
            let which = if self.eat_kw(Kw::Begin) {
                "begin"
            } else if self.eat_kw(Kw::End) {
                "end"
            } else {
                return self.err("expected 'begin' or 'end'");
            };
            let n = self.postfix_expr()?;
            parts.push(Expr::new(ExprKind::Call { func: Box::new(Expr::new(ExprKind::Ident(which.into()), loc)), args: vec![Arg::Pos(Some(n))], parens: true, with_: None }, loc));
            if !self.eat_kw(Kw::Or) {
                break;
            }
        }
        Ok(Expr::new(ExprKind::Concat(parts), loc))
    }

    fn cover_item(&mut self) -> Result<CoverItem> {
        let loc = self.loc();
        // option.x = v; / type_option.x = v;
        if matches!(self.peek(), T::Ident(n) if n == "option" || n == "type_option") && self.is_p_at(1, ".") {
            let T::Ident(n) = self.bump() else { unreachable!() };
            self.bump();
            let name = self.member_name()?;
            self.expect_p("=")?;
            let value = self.expr()?;
            self.expect_p(";")?;
            return Ok(CoverItem::Option { type_option: n == "type_option", name, value });
        }
        // [type] [label :] coverpoint / cross
        let mut ty = None;
        let mut label = None;
        if !matches!(self.kw(), Some(Kw::Coverpoint | Kw::Cross)) {
            if !(self.is_ident() && self.is_p_at(1, ":")) {
                ty = self.data_type()?;
            }
            label = Some(self.ident()?);
            self.expect_p(":")?;
        }
        if self.eat_kw(Kw::Coverpoint) {
            let expr = self.expr()?;
            let iff = self.opt_iff()?;
            let bins = self.bins_body()?;
            return Ok(CoverItem::Coverpoint(Box::new(Coverpoint { ty, label, expr, iff, bins, loc })));
        }
        self.expect_kw(Kw::Cross)?;
        let mut items = vec![self.postfix_expr()?];
        while self.eat_p(",") {
            items.push(self.postfix_expr()?);
        }
        let iff = self.opt_iff()?;
        let bins = self.bins_body()?;
        Ok(CoverItem::Cross(Box::new(Cross { label, items, iff, bins, loc })))
    }

    fn opt_iff(&mut self) -> Result<Option<Expr>> {
        if self.eat_kw(Kw::Iff) {
            self.expect_p("(")?;
            let e = self.expr()?;
            self.expect_p(")")?;
            return Ok(Some(e));
        }
        Ok(None)
    }

    /// `{ bins ... }` or `;`
    fn bins_body(&mut self) -> Result<Vec<BinsItem>> {
        let mut v = Vec::new();
        if self.eat_p(";") {
            return Ok(v);
        }
        self.expect_p("{")?;
        while !self.eat_p("}") {
            if self.at_eof() {
                return self.err("unexpected end of file in coverpoint");
            }
            if self.eat_p(";") {
                continue;
            }
            v.push(self.bins_item()?);
        }
        Ok(v)
    }

    fn bins_item(&mut self) -> Result<BinsItem> {
        let loc = self.loc();
        let empty = |loc| BinsItem {
            kind: None,
            wildcard: false,
            name: String::new(),
            array: None,
            values: BinsValues::Default,
            with_: None,
            iff: None,
            option: None,
            loc,
        };
        if matches!(self.peek(), T::Ident(n) if n == "option" || n == "type_option") && self.is_p_at(1, ".") {
            self.bump();
            self.bump();
            let n = self.member_name()?;
            self.expect_p("=")?;
            let e = self.expr()?;
            self.expect_p(";")?;
            let mut b = empty(loc);
            b.option = Some((n, e));
            return Ok(b);
        }
        // a function declared in a cross body
        if self.is_kw(Kw::Function) {
            self.subroutine(vec![])?;
            return Ok(empty(loc));
        }
        let wildcard = self.eat_kw(Kw::Wildcard);
        let kind = match self.bump() {
            T::Kw(k @ (Kw::Bins | Kw::IllegalBins | Kw::IgnoreBins)) => k,
            _ => {
                self.pos -= 1;
                return self.err(format!("expected bins, found {}", self.describe()));
            }
        };
        let name = self.ident()?;
        let array = if self.eat_p("[") {
            let n = if self.is_p("]") { None } else { Some(self.expr()?) };
            self.expect_p("]")?;
            Some(n)
        } else {
            None
        };
        self.expect_p("=")?;
        let values = if self.eat_kw(Kw::Default) {
            if self.eat_kw(Kw::Sequence) {
                BinsValues::DefaultSequence
            } else {
                BinsValues::Default
            }
        } else if self.is_p("{") {
            BinsValues::Ranges(self.range_set()?)
        } else if self.is_p("(") && self.trans_list_here() {
            let mut sets = Vec::new();
            loop {
                self.expect_p("(")?;
                let mut seq = Vec::new();
                loop {
                    let mut ranges = vec![self.value_range()?];
                    while self.eat_p(",") {
                        ranges.push(self.value_range()?);
                    }
                    let rep = if self.is_repetition_bracket() {
                        self.bump();
                        let k = match self.bump() {
                            T::Punct("*") => RepeatKind::Consecutive,
                            T::Punct("=") => RepeatKind::NonConsecutive,
                            _ => RepeatKind::Goto,
                        };
                        let a = self.expr()?;
                        let r = if self.eat_p(":") { CycleRange::Range(a, self.expr()?) } else { CycleRange::One(a) };
                        self.expect_p("]")?;
                        Some((k, r))
                    } else {
                        None
                    };
                    seq.push((ranges, rep));
                    if !self.eat_p("=>") {
                        break;
                    }
                }
                self.expect_p(")")?;
                sets.push(seq);
                if !self.eat_p(",") {
                    break;
                }
            }
            BinsValues::Trans(sets)
        } else {
            let saved = std::mem::replace(&mut self.in_constraint, false);
            let e = self.expr();
            self.in_constraint = saved;
            let e = e?;
            if matches!(&e.kind, ExprKind::Call { func, .. } if matches!(&func.kind, ExprKind::Ident(n) if n == "binsof"))
                || matches!(e.kind, ExprKind::Binary(..) | ExprKind::Unary(..))
            {
                BinsValues::Select(e)
            } else {
                BinsValues::Expr(e)
            }
        };
        let with_ = if self.eat_kw(Kw::With) {
            self.expect_p("(")?;
            let e = self.expr()?;
            self.expect_p(")")?;
            Some(e)
        } else {
            None
        };
        let iff = self.opt_iff()?;
        self.expect_p(";")?;
        Ok(BinsItem { kind: Some(kind), wildcard, name, array, values, with_, iff, option: None, loc })
    }

    /// `(` starts a transition list (`(1 => 2)`), not a parenthesized expression.
    fn trans_list_here(&self) -> bool {
        let mut depth = 0;
        let mut k = 0;
        loop {
            match self.peek_at(k) {
                T::Punct("(") => depth += 1,
                T::Punct(")") => {
                    depth -= 1;
                    if depth == 0 {
                        return false;
                    }
                }
                T::Punct("=>") if depth == 1 => return true,
                T::Punct("[") if depth == 1 && (self.is_p_at(k + 1, "*") || self.is_p_at(k + 1, "=") || self.is_p_at(k + 1, "->")) => return true,
                T::Eof => return false,
                _ => {}
            }
            k += 1;
        }
    }
}
