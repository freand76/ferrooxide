//! `randsequence` (IEEE 1800-2017 18.17), expanded into statements: each use of a production
//! is a named block holding its arguments and its rule (alternatives choose with the weights
//! as `randcase` does), `return` in a code block disables the production's block, `break`
//! the whole randsequence's. `rand join` interleaves the items of its productions at random.

use super::*;
use crate::syntax::tree as t;
use std::collections::HashMap;

/// The state of one expansion.
struct Rs<'p> {
    prods: HashMap<&'p str, &'p t::Production>,
    /// the block of the whole randsequence (`break`)
    top: String,
    /// the productions being expanded (a production that uses itself is not supported)
    stack: Vec<&'p str>,
}

fn st(kind: t::StmtKind, loc: t::Loc) -> t::Stmt {
    t::Stmt { kind, label: None, loc }
}

fn ident(n: &str, loc: t::Loc) -> t::Expr {
    t::Expr::new(t::ExprKind::Ident(n.to_string()), loc)
}

fn num(v: u64, loc: t::Loc) -> t::Expr {
    t::Expr::new(t::ExprKind::Num(crate::lexer::Number { size: None, signed: true, base: 10, based: false, digits: v.to_string(), fill: false }), loc)
}

fn block(name: Option<String>, decls: Vec<t::Item>, stmts: Vec<t::Stmt>, loc: t::Loc) -> t::Stmt {
    st(t::StmtKind::Block { fork: None, name, decls, stmts }, loc)
}

fn int_var(name: &str, init: Option<t::Expr>, loc: t::Loc) -> t::Item {
    let ty = t::DataType { kind: t::TypeKind::Int(t::IntKw::Int), signed: None, packed: vec![], loc };
    t::Item::Var(t::VarDecl { const_: false, var: false, lifetime: None, ty, decls: vec![t::Declarator { name: name.to_string(), dims: vec![], init, loc }], loc })
}

fn bin(op: t::BinOp, a: t::Expr, b: t::Expr) -> t::Expr {
    let loc = a.loc;
    t::Expr::new(t::ExprKind::Binary(op, Box::new(a), Box::new(b)), loc)
}

fn assign(lhs: t::Expr, op: t::AssignOp, rhs: t::Expr) -> t::Stmt {
    let loc = lhs.loc;
    st(t::StmtKind::Assign { lhs, op, timing: None, rhs }, loc)
}

impl Lower {
    pub(crate) fn randsequence(&mut self, start: Option<&String>, prods: &[t::Production], loc: t::Loc) -> Result<Stmt> {
        let mut map = HashMap::new();
        for p in prods {
            if map.insert(p.name.as_str(), p).is_some() {
                return self.err(p.loc, format!("production '{}' is declared twice", p.name));
            }
        }
        let Some(first) = prods.first() else { return self.err(loc, "a randsequence needs a production") };
        let main = match start {
            Some(s) => s.as_str(),
            None => first.name.as_str(),
        };
        self.spec_count += 1;
        let top = format!("rs${}", self.spec_count);
        let mut rs = Rs { prods: map, top: top.clone(), stack: vec![] };
        let body = self.rs_call(&mut rs, main, &[], loc)?;
        let s = block(Some(top), vec![], vec![body], loc);
        self.stmt(&s)
    }

    /// A use of production `name` with arguments `args`.
    fn rs_call<'p>(&mut self, rs: &mut Rs<'p>, name: &str, args: &[t::Arg], loc: t::Loc) -> Result<t::Stmt> {
        let Some(&p) = rs.prods.get(name) else {
            return self.err(loc, format!("unknown production '{name}'"));
        };
        if rs.stack.contains(&p.name.as_str()) {
            return self.unsupported(loc, &format!("a production that uses itself ('{name}')"));
        }
        if p.ret.as_ref().is_some_and(|r| !matches!(r.kind, t::TypeKind::Void)) {
            return self.unsupported(p.loc, "a production that returns a value");
        }
        self.spec_count += 1;
        let pname = format!("rs${}${name}", self.spec_count);
        // the arguments: variables of the production's block (IEEE 1800-2017 18.17.7)
        let ports = p.ports.as_deref().unwrap_or(&[]);
        if args.len() > ports.len() {
            return self.err(loc, format!("production '{name}' takes {} arguments", ports.len()));
        }
        let mut decls = Vec::new();
        let mut prev_ty: Option<t::DataType> = None;
        for (k, port) in ports.iter().enumerate() {
            if port.dir.is_some_and(|d| d != t::Dir::Input) {
                return self.unsupported(port.loc, "a production argument that is not an input");
            }
            let ty = match (&port.ty, &prev_ty) {
                (Some(ty), _) => ty.clone(),
                (None, Some(ty)) => ty.clone(),
                (None, None) => t::DataType { kind: t::TypeKind::Int(t::IntKw::Logic), signed: None, packed: vec![], loc: port.loc },
            };
            prev_ty = Some(ty.clone());
            let init = match args.get(k) {
                Some(t::Arg::Pos(Some(e))) => Some(e.clone()),
                Some(t::Arg::Named(..)) => return self.unsupported(loc, "a named production argument"),
                _ => port.default.clone(),
            };
            let Some(init) = init else { return self.err(loc, format!("argument '{}' of production '{name}' has no value", port.name)) };
            let d = t::Declarator { name: port.name.clone(), dims: port.dims.clone(), init: Some(init), loc: port.loc };
            decls.push(t::Item::Var(t::VarDecl { const_: false, var: false, lifetime: None, ty, decls: vec![d], loc: port.loc }));
        }
        rs.stack.push(&p.name);
        let body = self.rs_rules(rs, p, &pname);
        rs.stack.pop();
        Ok(block(Some(pname), decls, vec![body?], p.loc))
    }

    /// The rules of production `p` (`pname`: its block): one, or a choice among several.
    fn rs_rules<'p>(&mut self, rs: &mut Rs<'p>, p: &'p t::Production, pname: &str) -> Result<t::Stmt> {
        if let [(items, _)] = p.rules.as_slice() {
            return self.rs_items(rs, items, pname, p.loc);
        }
        let mut arms = Vec::new();
        for (items, w) in &p.rules {
            let body = self.rs_items(rs, items, pname, p.loc)?;
            arms.push((w.clone().unwrap_or_else(|| num(1, p.loc)), body));
        }
        Ok(st(t::StmtKind::RandCase(arms), p.loc))
    }

    fn rs_items<'p>(&mut self, rs: &mut Rs<'p>, items: &'p [t::ProdItem], pname: &str, loc: t::Loc) -> Result<t::Stmt> {
        let mut v = Vec::new();
        for it in items {
            v.push(self.rs_item(rs, it, pname, loc)?);
        }
        Ok(block(None, vec![], v, loc))
    }

    fn rs_item<'p>(&mut self, rs: &mut Rs<'p>, it: &'p t::ProdItem, pname: &str, loc: t::Loc) -> Result<t::Stmt> {
        Ok(match it {
            t::ProdItem::Call(n, args) => self.rs_call(rs, n, args, loc)?,
            t::ProdItem::Code(decls, stmts) => {
                let mut stmts = stmts.clone();
                for s in &mut stmts {
                    self.rs_aborts(s, pname, &rs.top, false)?;
                }
                block(None, decls.clone(), stmts, loc)
            }
            t::ProdItem::If { cond, then_, else_ } => {
                let then_ = Box::new(self.rs_item(rs, then_, pname, loc)?);
                let else_ = match else_ {
                    Some(e) => Some(Box::new(self.rs_item(rs, e, pname, loc)?)),
                    None => None,
                };
                st(t::StmtKind::If { qual: None, cond: cond.clone(), then_, else_ }, cond.loc)
            }
            t::ProdItem::Repeat { count, item } => {
                let body = Box::new(self.rs_item(rs, item, pname, loc)?);
                st(t::StmtKind::Repeat { count: count.clone(), body }, count.loc)
            }
            t::ProdItem::Case { expr, items } => {
                let mut v = Vec::new();
                for (labels, it) in items {
                    let body = self.rs_item(rs, it, pname, loc)?;
                    v.push(t::CaseItem { labels: labels.iter().cloned().map(t::ValueRange::Value).collect(), pattern: None, guard: None, body, loc: expr.loc });
                }
                st(t::StmtKind::Case { qual: None, kw: t::CaseKw::Case, expr: expr.clone(), mode: t::CaseMode::Normal, items: v }, expr.loc)
            }
            t::ProdItem::RandJoin { weight, items } => self.rs_join(rs, weight.as_ref(), items, pname, loc)?,
        })
    }

    /// `rand join a b ...` (IEEE 1800-2017 18.17.5): all items of the productions, those of
    /// each in order, interleaved at random (each interleaving equally likely; the weight
    /// expression is evaluated but does not bias the order).
    fn rs_join<'p>(&mut self, rs: &mut Rs<'p>, weight: Option<&t::Expr>, items: &'p [t::ProdItem], pname: &str, loc: t::Loc) -> Result<t::Stmt> {
        // the items of each production, expanded
        let mut lists: Vec<Vec<t::Stmt>> = Vec::new();
        for it in items {
            let t::ProdItem::Call(n, args) = it else { return self.err(loc, "rand join takes production names") };
            let Some(&p) = rs.prods.get(n.as_str()) else { return self.err(loc, format!("unknown production '{n}'")) };
            if !args.is_empty() || p.ports.as_ref().is_some_and(|v| !v.is_empty()) {
                return self.unsupported(loc, "a production with arguments in rand join");
            }
            if rs.stack.contains(&p.name.as_str()) {
                return self.unsupported(loc, &format!("a production that uses itself ('{n}')"));
            }
            let [(pitems, _)] = p.rules.as_slice() else {
                return self.unsupported(loc, "a production with alternatives in rand join");
            };
            rs.stack.push(&p.name);
            let mut v = Vec::new();
            for x in pitems {
                // (`return` ends the rand join's production: the rest of its items)
                if matches!(x, t::ProdItem::Code(_, s) if s.iter().any(has_return)) {
                    rs.stack.pop();
                    return self.unsupported(loc, "return in a production of rand join");
                }
                v.push(self.rs_item(rs, x, pname, loc)?);
            }
            rs.stack.pop();
            lists.push(v);
        }
        self.spec_count += 1;
        let k = self.spec_count;
        let left = format!("rs${k}$left");
        let pick = format!("rs${k}$pick");
        let total: usize = lists.iter().map(|v| v.len()).sum();
        let mut decls = vec![int_var(&left, Some(num(total as u64, loc)), loc), int_var(&pick, None, loc)];
        if let Some(w) = weight {
            decls.push(t::Item::Var(t::VarDecl {
                const_: false,
                var: false,
                lifetime: None,
                ty: t::DataType { kind: t::TypeKind::Real(t::RealKw::Real), signed: None, packed: vec![], loc },
                decls: vec![t::Declarator { name: format!("rs${k}$weight"), dims: vec![], init: Some(w.clone()), loc }],
                loc,
            }));
        }
        // per production: the items left, the next item
        let rem = |i: usize| format!("rs${k}$rem{i}");
        let pos = |i: usize| format!("rs${k}$pos{i}");
        for (i, v) in lists.iter().enumerate() {
            decls.push(int_var(&rem(i), Some(num(v.len() as u64, loc)), loc));
            decls.push(int_var(&pos(i), Some(num(0, loc)), loc));
        }
        // pick an item left (a production with probability by its items left), run the next
        // item of its production
        let mut chain: Option<t::Stmt> = None;
        let mut bounds = Vec::new();
        let mut acc: Option<t::Expr> = None;
        for i in 0..lists.len() {
            let r = ident(&rem(i), loc);
            let s = match acc {
                None => r,
                Some(a) => bin(t::BinOp::Add, a, r),
            };
            bounds.push(s.clone());
            acc = Some(s);
        }
        for (i, v) in lists.into_iter().enumerate().rev() {
            let arms: Vec<t::CaseItem> =
                v.into_iter().enumerate().map(|(j, body)| t::CaseItem { labels: vec![t::ValueRange::Value(num(j as u64, loc))], pattern: None, guard: None, body, loc }).collect();
            let run = block(
                None,
                vec![],
                vec![
                    st(t::StmtKind::Case { qual: None, kw: t::CaseKw::Case, expr: ident(&pos(i), loc), mode: t::CaseMode::Normal, items: arms }, loc),
                    assign(ident(&pos(i), loc), t::AssignOp::Add, num(1, loc)),
                    assign(ident(&rem(i), loc), t::AssignOp::Sub, num(1, loc)),
                ],
                loc,
            );
            let cond = bin(t::BinOp::Lt, ident(&pick, loc), bounds[i].clone());
            chain = Some(st(t::StmtKind::If { qual: None, cond, then_: Box::new(run), else_: chain.map(Box::new) }, loc));
        }
        let urandom = t::Expr::new(
            t::ExprKind::SysCall { name: "$urandom_range".into(), args: vec![t::Arg::Pos(Some(bin(t::BinOp::Sub, ident(&left, loc), num(1, loc)))), t::Arg::Pos(Some(num(0, loc)))], parens: true },
            loc,
        );
        let mut body = vec![assign(ident(&pick, loc), t::AssignOp::Eq, urandom)];
        body.extend(chain);
        body.push(assign(ident(&left, loc), t::AssignOp::Sub, num(1, loc)));
        let lp = st(t::StmtKind::While { cond: bin(t::BinOp::Gt, ident(&left, loc), num(0, loc)), body: Box::new(block(None, vec![], body, loc)) }, loc);
        Ok(block(None, decls, vec![lp], loc))
    }

    /// `return` and `break` in a code block (`in_loop`: inside a loop of it, where `break`
    /// is the loop's): disables of the production's block and of the randsequence's.
    fn rs_aborts(&self, s: &mut t::Stmt, prod: &str, top: &str, in_loop: bool) -> Result<()> {
        let loc = s.loc;
        match &mut s.kind {
            t::StmtKind::Return(Some(_)) => return self.unsupported(loc, "a production that returns a value"),
            t::StmtKind::Return(None) => s.kind = t::StmtKind::Disable(ident(prod, loc)),
            t::StmtKind::Break if !in_loop => s.kind = t::StmtKind::Disable(ident(top, loc)),
            t::StmtKind::Block { stmts, .. } => {
                for x in stmts {
                    self.rs_aborts(x, prod, top, in_loop)?;
                }
            }
            t::StmtKind::If { then_, else_, .. } => {
                self.rs_aborts(then_, prod, top, in_loop)?;
                if let Some(e) = else_ {
                    self.rs_aborts(e, prod, top, in_loop)?;
                }
            }
            t::StmtKind::Case { items, .. } => {
                for it in items {
                    self.rs_aborts(&mut it.body, prod, top, in_loop)?;
                }
            }
            t::StmtKind::RandCase(arms) => {
                for (_, b) in arms {
                    self.rs_aborts(b, prod, top, in_loop)?;
                }
            }
            t::StmtKind::Timed { body, .. } | t::StmtKind::Wait { body, .. } => self.rs_aborts(body, prod, top, in_loop)?,
            t::StmtKind::For { body, .. }
            | t::StmtKind::Foreach { body, .. }
            | t::StmtKind::While { body, .. }
            | t::StmtKind::DoWhile { body, .. }
            | t::StmtKind::Repeat { body, .. }
            | t::StmtKind::Forever(body) => self.rs_aborts(body, prod, top, true)?,
            _ => {}
        }
        Ok(())
    }
}

/// A `return` anywhere in a statement.
fn has_return(s: &t::Stmt) -> bool {
    match &s.kind {
        t::StmtKind::Return(_) => true,
        t::StmtKind::Block { stmts, .. } => stmts.iter().any(has_return),
        t::StmtKind::If { then_, else_, .. } => has_return(then_) || else_.as_deref().is_some_and(has_return),
        t::StmtKind::Case { items, .. } => items.iter().any(|i| has_return(&i.body)),
        t::StmtKind::RandCase(arms) => arms.iter().any(|(_, b)| has_return(b)),
        t::StmtKind::Timed { body, .. }
        | t::StmtKind::Wait { body, .. }
        | t::StmtKind::For { body, .. }
        | t::StmtKind::Foreach { body, .. }
        | t::StmtKind::While { body, .. }
        | t::StmtKind::DoWhile { body, .. }
        | t::StmtKind::Repeat { body, .. }
        | t::StmtKind::Forever(body) => has_return(body),
        _ => false,
    }
}
