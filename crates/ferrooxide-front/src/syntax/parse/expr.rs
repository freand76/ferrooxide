//! Expressions (IEEE 1800 A.8).

use super::types::{int_kw, is_type_kw};
use super::Parser;
use crate::syntax::kw::Kw;
use crate::syntax::token::T;
use crate::syntax::tree::*;
use crate::Result;

fn bx(e: Expr) -> Box<Expr> {
    Box::new(e)
}

impl Parser {
    /// An expression (no assignment).
    pub(crate) fn expr(&mut self) -> Result<Expr> {
        let loc = self.loc();
        let mut c = self.binary(1)?;
        if self.is_kw(Kw::Matches) && !self.in_constraint {
            self.bump();
            let p = self.pattern()?;
            c = Expr::new(ExprKind::Matches(bx(c), Box::new(p)), loc);
            while self.is_triple_and() {
                self.bump();
                self.bump();
                let r = self.binary(1)?;
                c = Expr::new(ExprKind::TripleAnd(bx(c), bx(r)), loc);
            }
        }
        let c = if self.eat_p("?") {
            let t = self.expr()?;
            self.expect_p(":")?;
            let f = self.expr()?;
            Expr::new(ExprKind::Cond(bx(c), bx(t), bx(f)), loc)
        } else {
            c
        };
        // `->` and `<->` bind below `?:` (right associative)
        if !self.in_constraint && (self.is_p("->") || self.is_p("<->")) {
            let op = if self.eat_p("->") {
                BinOp::LImpl
            } else {
                self.bump();
                BinOp::LEquiv
            };
            let r = self.expr()?;
            return Ok(Expr::new(ExprKind::Binary(op, bx(c), bx(r)), loc));
        }
        Ok(c)
    }

    /// An expression, or a data type where one may stand (`$bits(T)`, parameter values,
    /// `type(T)`).
    pub(crate) fn expr_or_type(&mut self) -> Result<Expr> {
        if self.is_type_only_here() {
            let loc = self.loc();
            let t = self.data_type()?.unwrap();
            return Ok(Expr::new(ExprKind::Type(Box::new(t)), loc));
        }
        self.expr()
    }

    /// A data type (not a cast or a call) starts here.
    fn is_type_only_here(&self) -> bool {
        match self.peek() {
            T::Kw(Kw::Type) => false,
            T::Kw(k) if is_type_kw(*k) || *k == Kw::Virtual => self.type_kw_ends_arg(),
            // `C #(..)::x` followed by more of an expression: a scoped name
            T::Ident(n) if self.is_type_name(n) && self.is_p_at(1, "#") && self.is_p_at(2, "(") => match self.after_parens(2) {
                Some(k) if self.is_p_at(k, "::") => matches!(self.peek_at(k + 2), T::Punct(")" | "," | "}")),
                _ => true,
            },
            T::Ident(n) if self.is_type_name(n) => matches!(self.peek_at(1), T::Punct(")" | "," | "[" | "#" | "}")) || self.is_kw_at(1, Kw::Signed) || self.is_kw_at(1, Kw::Unsigned),
            T::Ident(n) if self.is_p_at(1, "::") => {
                // `p::T` followed by the end of the argument
                matches!(self.peek_at(2), T::Ident(t) if self.packages.get(n.as_str()).is_some_and(|p| p.contains(t.as_str())))
                    && matches!(self.peek_at(3), T::Punct(")" | "," | "["))
            }
            _ => false,
        }
    }

    /// The position after the parentheses that open at `k` (None: they do not close).
    fn after_parens(&self, mut k: usize) -> Option<usize> {
        let mut depth = 0;
        loop {
            match self.peek_at(k) {
                T::Punct("(") => depth += 1,
                T::Punct(")") => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(k + 1);
                    }
                }
                T::Eof => return None,
                _ => {}
            }
            k += 1;
        }
    }

    /// A type keyword with signing and packed dimensions, then the end of an argument (`)`,
    /// `,`, `}`): `$bits(logic [3:0])`, but not `byte[3].x` (a name).
    fn type_kw_ends_arg(&self) -> bool {
        if matches!(self.peek(), T::Kw(Kw::Struct | Kw::Union | Kw::Enum | Kw::Virtual)) {
            return true;
        }
        let mut k = 1;
        if self.is_kw_at(k, Kw::Signed) || self.is_kw_at(k, Kw::Unsigned) {
            k += 1;
        }
        let mut depth = 0;
        loop {
            match self.peek_at(k) {
                T::Punct("[") => depth += 1,
                T::Punct("]") => depth -= 1,
                T::Punct(")" | "," | "}") if depth == 0 => return true,
                T::Eof => return false,
                _ if depth == 0 => return false,
                _ => {}
            }
            k += 1;
        }
    }

    /// An expression that may be an assignment (`for` steps, sequence match items,
    /// parenthesized assignments).
    pub(crate) fn expr_or_assign(&mut self) -> Result<Expr> {
        let loc = self.loc();
        if self.is_p("++") || self.is_p("--") {
            let inc = self.is_p("++");
            self.bump();
            let e = self.postfix_expr()?;
            return Ok(Expr::new(ExprKind::IncDec { prefix: true, inc, e: bx(e) }, loc));
        }
        let e = self.expr()?;
        if let Some(op) = self.assign_op() {
            self.bump();
            let r = self.expr()?;
            return Ok(Expr::new(ExprKind::Assign(op, bx(e), bx(r)), loc));
        }
        Ok(e)
    }

    /// `=` or a compound assignment operator at the cursor.
    pub(crate) fn assign_op(&self) -> Option<AssignOp> {
        Some(match self.peek() {
            T::Punct("=") => AssignOp::Eq,
            T::Punct("+=") => AssignOp::Add,
            T::Punct("-=") => AssignOp::Sub,
            T::Punct("*=") => AssignOp::Mul,
            T::Punct("/=") => AssignOp::Div,
            T::Punct("%=") => AssignOp::Mod,
            T::Punct("&=") => AssignOp::And,
            T::Punct("|=") => AssignOp::Or,
            T::Punct("^=") => AssignOp::Xor,
            T::Punct("<<=") => AssignOp::Shl,
            T::Punct(">>=") => AssignOp::Shr,
            T::Punct("<<<=") => AssignOp::AShl,
            T::Punct(">>>=") => AssignOp::AShr,
            _ => return None,
        })
    }

    fn binop(&self) -> Option<(BinOp, u8)> {
        let T::Punct(p) = self.peek() else { return None };
        Some(match *p {
            "||" => (BinOp::LOr, 1),
            "&&" => (BinOp::LAnd, 2),
            "|" => (BinOp::Or, 3),
            "^" => (BinOp::Xor, 4),
            "~^" | "^~" => (BinOp::Xnor, 4),
            "&" => (BinOp::And, 5),
            "==" => (BinOp::Eq, 6),
            "!=" => (BinOp::Ne, 6),
            "===" => (BinOp::CaseEq, 6),
            "!==" => (BinOp::CaseNe, 6),
            "==?" => (BinOp::WildEq, 6),
            "!=?" => (BinOp::WildNe, 6),
            "<" => (BinOp::Lt, 7),
            "<=" => (BinOp::Le, 7),
            ">" => (BinOp::Gt, 7),
            ">=" => (BinOp::Ge, 7),
            "<<" => (BinOp::Shl, 8),
            ">>" => (BinOp::Shr, 8),
            "<<<" => (BinOp::AShl, 8),
            ">>>" => (BinOp::AShr, 8),
            "+" => (BinOp::Add, 9),
            "-" => (BinOp::Sub, 9),
            "*" => (BinOp::Mul, 10),
            "/" => (BinOp::Div, 10),
            "%" => (BinOp::Mod, 10),
            "**" => (BinOp::Pow, 11),
            _ => return None,
        })
    }

    fn binary(&mut self, min_prec: u8) -> Result<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let loc = lhs.loc;
            // `inside` and `dist` bind like the relational operators
            if min_prec <= 7 && self.is_kw(Kw::Inside) {
                self.bump();
                let set = self.range_set()?;
                lhs = Expr::new(ExprKind::Inside(bx(lhs), set), loc);
                continue;
            }
            // `binsof(x) intersect {...}` in cross bins (kept as a set membership)
            if min_prec <= 7 && self.is_kw(Kw::Intersect) && self.is_p_at(1, "{") {
                self.bump();
                let set = self.range_set()?;
                lhs = Expr::new(ExprKind::Inside(bx(lhs), set), loc);
                continue;
            }
            if min_prec <= 7 && self.is_kw(Kw::Dist) {
                self.bump();
                let items = self.dist_items()?;
                lhs = Expr::new(ExprKind::Dist(bx(lhs), items), loc);
                continue;
            }
            let Some((op, prec)) = self.binop() else { break };
            if prec < min_prec {
                break;
            }
            self.bump();
            // `**` is right associative here (as the old parser)
            let next = if op == BinOp::Pow { prec } else { prec + 1 };
            let rhs = self.binary(next)?;
            lhs = Expr::new(ExprKind::Binary(op, bx(lhs), bx(rhs)), loc);
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr> {
        let loc = self.loc();
        let op = match self.peek() {
            T::Punct("+") => Some(UnOp::Plus),
            T::Punct("-") => Some(UnOp::Neg),
            T::Punct("~") => Some(UnOp::Not),
            T::Punct("!") => Some(UnOp::LNot),
            T::Punct("&") => Some(UnOp::RedAnd),
            T::Punct("|") => Some(UnOp::RedOr),
            T::Punct("^") => Some(UnOp::RedXor),
            T::Punct("~&") => Some(UnOp::RedNand),
            T::Punct("~|") => Some(UnOp::RedNor),
            T::Punct("~^" | "^~") => Some(UnOp::RedXnor),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let e = self.unary()?;
            return Ok(Expr::new(ExprKind::Unary(op, bx(e)), loc));
        }
        if self.is_p("++") || self.is_p("--") {
            let inc = self.is_p("++");
            self.bump();
            let e = self.unary()?;
            return Ok(Expr::new(ExprKind::IncDec { prefix: true, inc, e: bx(e) }, loc));
        }
        let e = self.primary()?;
        if self.is_p("++") || self.is_p("--") {
            let inc = self.is_p("++");
            self.bump();
            return Ok(Expr::new(ExprKind::IncDec { prefix: false, inc, e: bx(e) }, loc));
        }
        Ok(e)
    }

    /// A primary with its selects, members and calls, but no operators (assignment targets).
    pub(crate) fn postfix_expr(&mut self) -> Result<Expr> {
        self.primary()
    }

    pub(crate) fn primary(&mut self) -> Result<Expr> {
        let loc = self.loc();
        let t = self.peek().clone();
        let e = match t {
            T::Num(n) => {
                self.bump();
                let e = Expr::new(ExprKind::Num(n), loc);
                return self.size_cast(e);
            }
            T::Real(r) => {
                self.bump();
                Expr::new(ExprKind::Real(r), loc)
            }
            T::Time(v, u) => {
                self.bump();
                Expr::new(ExprKind::Time(v, u), loc)
            }
            T::Step => {
                self.bump();
                Expr::new(ExprKind::Step, loc)
            }
            T::Str(s) => {
                self.bump();
                Expr::new(ExprKind::Str(s), loc)
            }
            T::Punct("'") if self.is_p_at(1, "{") => {
                self.bump();
                let items = self.pattern_items()?;
                Expr::new(ExprKind::Pattern { ty: None, items }, loc)
            }
            T::Punct("(") => {
                self.bump();
                let e = self.expr()?;
                let e = if self.eat_p(":") {
                    let t = self.expr()?;
                    self.expect_p(":")?;
                    let m = self.expr()?;
                    Expr::new(ExprKind::MinTypMax(bx(e), bx(t), bx(m)), loc)
                } else if let Some(op) = self.assign_op() {
                    self.bump();
                    let r = self.expr()?;
                    Expr::new(ExprKind::Assign(op, bx(e), bx(r)), loc)
                } else {
                    e
                };
                self.expect_p(")")?;
                return self.size_cast(e);
            }
            T::Punct("{") => {
                let c = self.concat()?;
                return self.postfix(c);
            }
            T::Punct("$") => {
                self.bump();
                Expr::new(ExprKind::Dollar, loc)
            }
            T::SysIdent(n) if n == "$root" => {
                self.bump();
                Expr::new(ExprKind::Root, loc)
            }
            T::SysIdent(n) if n == "$unit" && self.is_p_at(1, "::") => {
                let name = self.scoped_name()?;
                Expr::new(ExprKind::Scoped(Box::new(name)), loc)
            }
            T::SysIdent(name) => {
                self.bump();
                let (args, parens) = if self.is_p("(") { (self.args()?, true) } else { (Vec::new(), false) };
                let e = Expr::new(ExprKind::SysCall { name, args, parens }, loc);
                // `$cast(...)` & co. take no postfix; `$root`-like names were handled above
                return self.size_cast(e);
            }
            T::Kw(Kw::Null) => {
                self.bump();
                Expr::new(ExprKind::Null, loc)
            }
            T::Kw(Kw::This) => {
                self.bump();
                Expr::new(ExprKind::This, loc)
            }
            T::Kw(Kw::Super) => {
                self.bump();
                Expr::new(ExprKind::Super, loc)
            }
            T::Kw(Kw::New) => return self.new_expr(),
            T::Kw(Kw::Tagged) => {
                self.bump();
                let name = self.member_name()?;
                let v = if self.starts_primary() { Some(bx(self.primary()?)) } else { None };
                return Ok(Expr::new(ExprKind::Tagged(name, v), loc));
            }
            T::Kw(Kw::Type) if self.is_p_at(1, "(") => {
                let t = self.data_type()?.unwrap();
                if self.is_p("'") {
                    return self.cast_after_type(t);
                }
                Expr::new(ExprKind::Type(Box::new(t)), loc)
            }
            T::Kw(k @ (Kw::Signed | Kw::Unsigned | Kw::Const)) if self.is_p_at(1, "'") => {
                self.bump();
                self.bump();
                let target = match k {
                    Kw::Signed => CastTarget::Signed,
                    Kw::Unsigned => CastTarget::Unsigned,
                    _ => CastTarget::Const,
                };
                self.expect_p("(")?;
                let e = self.expr()?;
                self.expect_p(")")?;
                return Ok(Expr::new(ExprKind::Cast(target, bx(e)), loc));
            }
            // `int'(e)`, `logic [3:0]'(e)`, `string'(e)`
            T::Kw(k) if is_type_kw(k) && !matches!(k, Kw::Struct | Kw::Union | Kw::Enum) && self.type_cast_follows() => {
                let t = self.data_type()?.unwrap();
                return self.cast_after_type(t);
            }
            T::Kw(Kw::Local) if self.is_p_at(1, "::") => {
                let name = self.scoped_name()?;
                Expr::new(ExprKind::Scoped(Box::new(name)), loc)
            }
            T::Kw(k) if !k.is_hard() && int_kw(k).is_none_or(|_| !self.is_p_at(1, "'")) => {
                // a keyword IEEE 1800 added, used as a name
                self.bump();
                Expr::new(ExprKind::Ident(k.as_str().to_string()), loc)
            }
            T::Ident(n) => {
                // `T'(e)`, `T'{...}`
                if self.is_p_at(1, "'") && (self.is_p_at(2, "(") || self.is_p_at(2, "{")) && self.is_type_name(&n) {
                    let t = self.data_type_named()?;
                    return self.cast_after_type(t);
                }
                if self.is_p_at(1, "::") || (self.is_p_at(1, "#") && self.is_p_at(2, "(") && self.is_type_name(&n)) {
                    let mut name = self.scoped_name()?;
                    if self.is_p("'") && (self.is_p_at(1, "(") || self.is_p_at(1, "{")) {
                        let t = DataType { kind: TypeKind::Named(Box::new(name)), signed: None, packed: vec![], loc };
                        return self.cast_after_type(t);
                    }
                    if name.name == "new" {
                        // `C::new(args)`: the class is the name before `::new`
                        let last = name.scopes.pop().unwrap();
                        let class = Name { scopes: name.scopes, name: last.name, params: last.params, loc };
                        let args = if self.is_p("(") { Some(self.args()?) } else { None };
                        return Ok(Expr::new(ExprKind::TypedNew { class: Box::new(class), args }, loc));
                    }
                    Expr::new(ExprKind::Scoped(Box::new(name)), loc)
                } else {
                    self.bump();
                    Expr::new(ExprKind::Ident(n), loc)
                }
            }
            _ => return self.err(format!("unexpected {} in expression", self.describe())),
        };
        self.postfix(e)
    }

    /// A primary can start here (for `tagged Name expr`).
    fn starts_primary(&self) -> bool {
        match self.peek() {
            T::Num(_) | T::Real(_) | T::Time(..) | T::Str(_) | T::Ident(_) | T::SysIdent(_) => true,
            T::Punct("(" | "{" | "'") => true,
            T::Kw(k) => matches!(k, Kw::Tagged | Kw::Null | Kw::This),
            _ => false,
        }
    }

    /// A type keyword with optional signing and packed dimensions, then `'`.
    fn type_cast_follows(&self) -> bool {
        let mut k = 1;
        if self.is_kw_at(k, Kw::Signed) || self.is_kw_at(k, Kw::Unsigned) {
            k += 1;
        }
        let mut depth = 0;
        loop {
            match self.peek_at(k) {
                T::Punct("[") => depth += 1,
                T::Punct("]") => depth -= 1,
                T::Punct("'") if depth == 0 => return true,
                T::Eof => return false,
                _ if depth == 0 => return false,
                _ => {}
            }
            k += 1;
        }
    }

    /// A type name with signing and dimensions (the cursor is at the name).
    fn data_type_named(&mut self) -> Result<DataType> {
        let loc = self.loc();
        let name = self.type_name()?;
        let signed = self.signing();
        let packed = self.packed_dims()?;
        Ok(DataType { kind: TypeKind::Named(Box::new(name)), signed, packed, loc })
    }

    /// After a type, at `'`: a cast `T'(e)` or a typed pattern `T'{...}`.
    fn cast_after_type(&mut self, t: DataType) -> Result<Expr> {
        let loc = t.loc;
        self.expect_p("'")?;
        if self.is_p("{") {
            let items = self.pattern_items()?;
            return Ok(Expr::new(ExprKind::Pattern { ty: Some(Box::new(t)), items }, loc));
        }
        self.expect_p("(")?;
        let e = self.expr()?;
        self.expect_p(")")?;
        Ok(Expr::new(ExprKind::Cast(CastTarget::Type(Box::new(t)), bx(e)), loc))
    }

    /// `e'(x)` after a size expression (a number, a parenthesized expression, a name).
    fn size_cast(&mut self, e: Expr) -> Result<Expr> {
        if self.is_p("'") && self.is_p_at(1, "(") {
            let loc = e.loc;
            self.bump();
            self.bump();
            let x = self.expr()?;
            self.expect_p(")")?;
            return Ok(Expr::new(ExprKind::Cast(CastTarget::Size(bx(e)), bx(x)), loc));
        }
        Ok(e)
    }

    /// Selects, members, calls and `with` after a name.
    fn postfix(&mut self, mut e: Expr) -> Result<Expr> {
        loop {
            let loc = e.loc;
            if self.is_p(".") && !self.is_p_at(1, "*") {
                self.bump();
                let m = self.member_name()?;
                e = Expr::new(ExprKind::Member(bx(e), m), loc);
                continue;
            }
            if self.is_p("[") && !self.is_repetition_bracket() {
                self.bump();
                let a = self.expr()?;
                let kind = if self.eat_p(":") {
                    Some(RangeKind::Colon)
                } else if self.eat_p("+:") {
                    Some(RangeKind::Plus)
                } else if self.eat_p("-:") {
                    Some(RangeKind::Minus)
                } else {
                    None
                };
                e = match kind {
                    Some(k) => {
                        let b = self.expr()?;
                        Expr::new(ExprKind::Range(bx(e), k, bx(a), bx(b)), loc)
                    }
                    None => Expr::new(ExprKind::Index(bx(e), bx(a)), loc),
                };
                self.expect_p("]")?;
                continue;
            }
            if self.is_p("(") && matches!(e.kind, ExprKind::Ident(_) | ExprKind::Member(..) | ExprKind::Scoped(_)) {
                let args = self.args()?;
                e = Expr::new(ExprKind::Call { func: bx(e), args, parens: true, with_: None }, loc);
                continue;
            }
            if self.is_kw(Kw::With) && (self.is_p_at(1, "(") || self.is_p_at(1, "{")) && matches!(e.kind, ExprKind::Call { .. } | ExprKind::Member(..) | ExprKind::Ident(_)) {
                self.bump();
                let w = self.with_clause()?;
                e = match e.kind {
                    ExprKind::Call { func, args, parens, .. } => {
                        Expr::new(ExprKind::Call { func, args, parens, with_: Some(Box::new(w)) }, loc)
                    }
                    k => Expr::new(ExprKind::Call { func: bx(Expr::new(k, loc)), args: vec![], parens: false, with_: Some(Box::new(w)) }, loc),
                };
                continue;
            }
            if self.is_p("'") && self.is_p_at(1, "(") {
                return self.size_cast(e);
            }
            if self.is_p("'") && self.is_p_at(1, "{") {
                // `T'{...}` with a type name the parser does not know (from elsewhere)
                if let ExprKind::Ident(n) = &e.kind {
                    let t = DataType {
                        kind: TypeKind::Named(Box::new(Name { scopes: vec![], name: n.clone(), params: None, loc })),
                        signed: None,
                        packed: vec![],
                        loc,
                    };
                    return self.cast_after_type(t);
                }
            }
            return Ok(e);
        }
    }

    /// `[*`, `[=`, `[->`, `[+]`: a sequence repetition, not a select.
    pub(crate) fn is_repetition_bracket(&self) -> bool {
        self.is_p("[") && (self.is_p_at(1, "*") || self.is_p_at(1, "=") || self.is_p_at(1, "->") || (self.is_p_at(1, "+") && self.is_p_at(2, "]")))
    }

    /// `with (expr)` or `with [(names)] { constraints }`.
    fn with_clause(&mut self) -> Result<With> {
        if self.is_p("(") {
            // `with (a, b) { ... }`: names for randomize
            let save = self.pos;
            self.bump();
            let mut names = Vec::new();
            let mut only_names = true;
            loop {
                if self.is_ident() && (self.is_p_at(1, ",") || self.is_p_at(1, ")")) {
                    names.push(self.ident()?);
                    if self.eat_p(",") {
                        continue;
                    }
                    break;
                }
                only_names = self.is_p(")") && names.is_empty();
                break;
            }
            if only_names && self.eat_p(")") && self.is_p("{") {
                let items = self.constraint_block()?;
                return Ok(With::Constraint(Some(names), items));
            }
            self.pos = save;
            self.bump();
            let e = self.expr()?;
            self.expect_p(")")?;
            return Ok(With::Expr(e));
        }
        let items = self.constraint_block()?;
        Ok(With::Constraint(None, items))
    }

    /// `new`, `new(args)`, `new[n]`, `new[n](init)`, `new obj`
    fn new_expr(&mut self) -> Result<Expr> {
        let loc = self.loc();
        self.expect_kw(Kw::New)?;
        let mut size = None;
        let mut args = None;
        let mut copy = None;
        if self.eat_p("[") {
            size = Some(bx(self.expr()?));
            self.expect_p("]")?;
        }
        if self.is_p("(") {
            args = Some(self.args()?);
        } else if size.is_none() && (self.is_ident() || self.is_kw(Kw::This)) && !self.is_p_at(1, "(") {
            copy = Some(bx(self.primary()?));
        }
        Ok(Expr::new(ExprKind::New { size, args, copy }, loc))
    }

    /// `name::name...` with optional class parameters (`C#(int)::x`), `$unit::x`, `local::x`.
    pub(crate) fn scoped_name(&mut self) -> Result<Name> {
        let loc = self.loc();
        let mut scopes = Vec::new();
        let mut name = match self.bump() {
            T::Ident(s) | T::SysIdent(s) => s,
            T::Kw(k) => k.as_str().to_string(),
            _ => {
                self.pos -= 1;
                return self.err("expected a name");
            }
        };
        loop {
            let params = if self.is_p("#") && self.is_p_at(1, "(") { Some(self.param_values()?) } else { None };
            if self.eat_p("::") {
                scopes.push(NamePart { name, params });
                name = match self.bump() {
                    T::Ident(s) => s,
                    T::Kw(k) => k.as_str().to_string(),
                    _ => {
                        self.pos -= 1;
                        return self.err(format!("expected a name after '::', found {}", self.describe()));
                    }
                };
                continue;
            }
            return Ok(Name { scopes, name, params, loc });
        }
    }

    /// Arguments `( ... )` of a call: positional (possibly empty) and `.name(e)`.
    pub(crate) fn args(&mut self) -> Result<Vec<Arg>> {
        self.expect_p("(")?;
        let mut v = Vec::new();
        if self.eat_p(")") {
            return Ok(v);
        }
        loop {
            if self.is_p(".") && self.is_ident_at(1) {
                self.bump();
                let n = self.ident()?;
                self.expect_p("(")?;
                let e = if self.is_p(")") { None } else { Some(self.expr_or_type()?) };
                self.expect_p(")")?;
                v.push(Arg::Named(n, e));
            } else if self.is_p(",") || self.is_p(")") {
                v.push(Arg::Pos(None));
            } else {
                v.push(Arg::Pos(Some(self.expr_or_type()?)));
            }
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        Ok(v)
    }

    /// `#(values)` or `#value` (a parameter value assignment), the cursor at `#`.
    pub(crate) fn param_values(&mut self) -> Result<Vec<ParamArg>> {
        self.expect_p("#")?;
        if !self.is_p("(") {
            // `#8`, `#W`, `#1.5`
            let e = self.delay_primary()?;
            return Ok(vec![ParamArg::Positional(e)]);
        }
        self.bump();
        let mut v = Vec::new();
        if self.eat_p(")") {
            return Ok(v);
        }
        loop {
            if self.is_p(".") {
                self.bump();
                let n = self.ident()?;
                self.expect_p("(")?;
                let e = if self.is_p(")") { None } else { Some(self.expr_or_type()?) };
                self.expect_p(")")?;
                v.push(ParamArg::Named(n, e));
            } else if self.is_p(",") || self.is_p(")") {
                v.push(ParamArg::Empty);
            } else {
                v.push(ParamArg::Positional(self.expr_or_type()?));
            }
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        Ok(v)
    }

    /// A delay or parameter value without parentheses: a number, a time, a name.
    pub(crate) fn delay_primary(&mut self) -> Result<Expr> {
        let loc = self.loc();
        let k = match self.peek().clone() {
            T::Num(n) => ExprKind::Num(n),
            T::Real(r) => ExprKind::Real(r),
            T::Time(v, u) => ExprKind::Time(v, u),
            T::Step => ExprKind::Step,
            T::Ident(_) if self.is_p_at(1, "::") => {
                let n = self.scoped_name()?;
                return Ok(Expr::new(ExprKind::Scoped(Box::new(n)), loc));
            }
            T::Ident(s) => ExprKind::Ident(s),
            T::Kw(k) if !k.is_hard() => ExprKind::Ident(k.as_str().to_string()),
            _ => return self.err(format!("bad delay value {}", self.describe())),
        };
        self.bump();
        Ok(Expr::new(k, loc))
    }

    /// `{...}`: concatenation, replication, streaming, or the empty queue `{}`.
    fn concat(&mut self) -> Result<Expr> {
        let loc = self.loc();
        self.expect_p("{")?;
        if self.eat_p("}") {
            return Ok(Expr::new(ExprKind::Concat(vec![]), loc));
        }
        if (self.is_p("<<") || self.is_p(">>")) && !self.is_p_at(1, "}") {
            let left = self.is_p("<<");
            self.bump();
            let slice = if self.is_p("{") { None } else { Some(bx(self.expr_or_type()?)) };
            self.expect_p("{")?;
            let mut items = Vec::new();
            loop {
                let e = self.expr()?;
                let w = if self.eat_kw(Kw::With) {
                    self.expect_p("[")?;
                    let loc = self.loc();
                    let a = self.expr()?;
                    let r = if self.eat_p(":") {
                        let b = self.expr()?;
                        Expr::new(ExprKind::Range(bx(Expr::new(ExprKind::Null, loc)), RangeKind::Colon, bx(a), bx(b)), loc)
                    } else if self.eat_p("+:") {
                        let b = self.expr()?;
                        Expr::new(ExprKind::Range(bx(Expr::new(ExprKind::Null, loc)), RangeKind::Plus, bx(a), bx(b)), loc)
                    } else if self.eat_p("-:") {
                        let b = self.expr()?;
                        Expr::new(ExprKind::Range(bx(Expr::new(ExprKind::Null, loc)), RangeKind::Minus, bx(a), bx(b)), loc)
                    } else {
                        a
                    };
                    self.expect_p("]")?;
                    Some(bx(r))
                } else {
                    None
                };
                items.push((e, w));
                if !self.eat_p(",") {
                    break;
                }
            }
            self.expect_p("}")?;
            self.expect_p("}")?;
            return Ok(Expr::new(ExprKind::Stream { left, slice, items }, loc));
        }
        let first = self.expr()?;
        if self.is_p("{") {
            self.bump();
            let mut v = Vec::new();
            if !self.is_p("}") {
                loop {
                    v.push(self.expr()?);
                    if !self.eat_p(",") {
                        break;
                    }
                }
            }
            self.expect_p("}")?;
            self.expect_p("}")?;
            return Ok(Expr::new(ExprKind::Repl(bx(first), v), loc));
        }
        let mut v = vec![first];
        while self.eat_p(",") {
            v.push(self.expr()?);
        }
        self.expect_p("}")?;
        Ok(Expr::new(ExprKind::Concat(v), loc))
    }

    /// `{...}` of an assignment pattern, at the `{`.
    pub(crate) fn pattern_items(&mut self) -> Result<PatternItems> {
        self.expect_p("{")?;
        if self.eat_p("}") {
            return Ok(PatternItems::Positional(vec![]));
        }
        let loc = self.loc();
        let first_key = if self.is_kw(Kw::Default) && self.is_p_at(1, ":") {
            self.bump();
            Some(Expr::new(ExprKind::Default, loc))
        } else {
            None
        };
        let first = match first_key {
            Some(k) => k,
            None => self.expr_or_type()?,
        };
        if self.is_p("{") {
            // `'{n{a, b}}`
            self.bump();
            let mut v = Vec::new();
            loop {
                v.push(self.expr()?);
                if !self.eat_p(",") {
                    break;
                }
            }
            self.expect_p("}")?;
            self.expect_p("}")?;
            return Ok(PatternItems::Repl(bx(first), v));
        }
        if self.eat_p(":") {
            let v = self.expr()?;
            let mut items = vec![(first, v)];
            while self.eat_p(",") {
                let loc = self.loc();
                let k = if self.eat_kw(Kw::Default) { Expr::new(ExprKind::Default, loc) } else { self.expr_or_type()? };
                self.expect_p(":")?;
                items.push((k, self.expr()?));
            }
            self.expect_p("}")?;
            return Ok(PatternItems::Keyed(items));
        }
        let mut v = vec![first];
        while self.eat_p(",") {
            v.push(self.expr()?);
        }
        self.expect_p("}")?;
        Ok(PatternItems::Positional(v))
    }

    /// One value or `[lo:hi]` range of a set (`inside`, `dist`, `unique`, case-inside items).
    pub(crate) fn value_range(&mut self) -> Result<ValueRange> {
        if self.eat_p("[") {
            let a = self.expr()?;
            self.expect_p(":")?;
            let b = self.expr()?;
            self.expect_p("]")?;
            return Ok(ValueRange::Range(a, b));
        }
        Ok(ValueRange::Value(self.expr()?))
    }

    /// `{ range, ... }`
    pub(crate) fn range_set(&mut self) -> Result<Vec<ValueRange>> {
        self.expect_p("{")?;
        let mut v = Vec::new();
        loop {
            v.push(self.value_range()?);
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p("}")?;
        Ok(v)
    }

    fn dist_items(&mut self) -> Result<Vec<DistItem>> {
        self.expect_p("{")?;
        let mut v = Vec::new();
        loop {
            let range = if self.is_kw(Kw::Default) {
                let loc = self.loc();
                self.bump();
                ValueRange::Value(Expr::new(ExprKind::Default, loc))
            } else {
                self.value_range()?
            };
            let weight = if self.eat_p(":=") {
                Some((false, self.expr()?))
            } else if self.eat_p(":/") {
                Some((true, self.expr()?))
            } else {
                None
            };
            v.push(DistItem { range, weight });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p("}")?;
        Ok(v)
    }

    /// The condition of `if`: an expression, possibly with `matches pattern` and `&&& e`.
    pub(crate) fn cond_predicate(&mut self) -> Result<Expr> {
        let loc = self.loc();
        let mut e = self.expr()?;
        if self.eat_kw(Kw::Matches) {
            let p = self.pattern()?;
            e = Expr::new(ExprKind::Matches(bx(e), Box::new(p)), loc);
        }
        while self.is_triple_and() {
            self.bump();
            self.bump();
            let mut r = self.expr()?;
            if self.eat_kw(Kw::Matches) {
                let p = self.pattern()?;
                r = Expr::new(ExprKind::Matches(bx(r), Box::new(p)), loc);
            }
            e = Expr::new(ExprKind::TripleAnd(bx(e), bx(r)), loc);
        }
        Ok(e)
    }

    /// `&&&` (lexed as `&&` and `&`, adjacent)
    pub(crate) fn is_triple_and(&self) -> bool {
        self.is_p("&&") && self.is_p_at(1, "&") && self.toks[self.pos].end == self.toks[self.pos + 1].pos
    }

    /// A pattern of `matches` (IEEE 1800 12.6).
    pub(crate) fn pattern(&mut self) -> Result<Pattern> {
        if self.is_p(".") {
            self.bump();
            if self.eat_p("*") {
                return Ok(Pattern::Wild);
            }
            return Ok(Pattern::Var(self.ident()?));
        }
        if self.is_p(".*") {
            self.bump();
            return Ok(Pattern::Wild);
        }
        if self.eat_kw(Kw::Tagged) {
            let n = self.member_name()?;
            let sub = if self.is_p("(") || self.is_p(".") || self.is_p("'") || self.starts_primary() {
                Some(Box::new(self.pattern()?))
            } else {
                None
            };
            return Ok(Pattern::Tagged(n, sub));
        }
        if self.is_p("'") && self.is_p_at(1, "{") {
            self.bump();
            self.bump();
            let mut v = Vec::new();
            loop {
                let key = if self.is_ident() && self.is_p_at(1, ":") {
                    let k = self.ident()?;
                    self.bump();
                    Some(k)
                } else {
                    None
                };
                v.push((key, self.pattern()?));
                if !self.eat_p(",") {
                    break;
                }
            }
            self.expect_p("}")?;
            return Ok(Pattern::List(v));
        }
        if self.is_p("(") {
            // a parenthesized pattern
            let save = self.pos;
            self.bump();
            if let Some(p) = self.attempt(|p| {
                let x = p.pattern()?;
                p.expect_p(")")?;
                Ok(x)
            }) {
                return Ok(p);
            }
            self.pos = save;
        }
        Ok(Pattern::Expr(self.binary(1)?))
    }
}
