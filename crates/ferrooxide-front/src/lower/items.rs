//! Module items: declarations, parameters, continuous assignments, processes, subroutines,
//! instances and gates, generate constructs, specify blocks.

use super::{item_loc, item_what, Lower};
use crate::ast::*;

use crate::syntax::kw::Kw;
use crate::syntax::tree as t;
use crate::Result;

impl Lower {
    /// A list of items in source order (`rest` of each is what follows it in the scope).
    pub(crate) fn item_list(&mut self, v: &[&t::Item], out: &mut Vec<Item>) -> Result<()> {
        for (k, it) in v.iter().enumerate() {
            self.module_item(it, &v[k + 1..], out)?;
        }
        Ok(())
    }

    fn gen_items(&mut self, b: &t::GenBlock) -> Result<(Option<String>, Vec<Item>)> {
        let name = b.name.as_ref().map(|n| self.name(n, b.loc));
        let mut items = Vec::new();
        let refs: Vec<&t::Item> = b.items.iter().collect();
        self.item_list(&refs, &mut items)?;
        Ok((name, items))
    }

    pub(crate) fn dir(&self, d: t::Dir, loc: t::Loc) -> Result<Dir> {
        match d {
            t::Dir::Input => Ok(Dir::Input),
            t::Dir::Output => Ok(Dir::Output),
            t::Dir::Inout => Ok(Dir::Inout),
            _ => self.unsupported(loc, "a ref port"),
        }
    }

    /// A delay `#d` / `#(d)` / `#(rise, fall, off)` (min:typ:max takes typ).
    pub(crate) fn delay(&mut self, d: &t::Delay) -> Result<Expr> {
        match d {
            t::Delay::Value(e) => self.expr(e),
            t::Delay::List(v) => {
                let mut out = Vec::new();
                for e in v {
                    out.push(self.expr(e)?);
                }
                Ok(Expr::SysCall("$__delays".into(), out))
            }
        }
    }

    pub(crate) fn module_item(&mut self, it: &t::Item, rest: &[&t::Item], items: &mut Vec<Item>) -> Result<()> {
        if self.in_program {
            // (IEEE 1800-2017 24.3)
            match it {
                t::Item::Process(p) if !matches!(p.kind, t::ProcKind::Initial | t::ProcKind::Final) => {
                    return self.err(p.loc, "Always statements are not allowed in program blocks.");
                }
                t::Item::Instances(_) => return self.err(item_loc(it), "Module instantiations are not allowed in program blocks."),
                t::Item::Gates(_) => {
                    return self.err(item_loc(it), "Gates and switches may not be instantiated in program blocks.");
                }
                _ => {}
            }
        }
        match it {
            t::Item::Empty(_) => {}
            t::Item::Directive(d, _) => self.directive(d),
            t::Item::Timeunits { unit, precision, loc } => {
                let (u, p) = self.timeunits(*unit, *precision, *loc)?;
                self.local_tu = (u.or(self.local_tu.0), p.or(self.local_tu.1));
            }
            t::Item::Param(p) => {
                let local = p.kw != Some(Kw::Parameter);
                match &p.kind {
                    t::ParamKind::Type { assigns } => {
                        for (n, d, loc) in assigns {
                            let it = self.type_param(local, n, d.as_ref(), *loc)?;
                            items.push(it);
                        }
                    }
                    t::ParamKind::Value { ty, assigns } => {
                        let pt = self.param_type(ty)?;
                        for d in assigns {
                            let p = self.param_assign(local, &pt, d)?;
                            items.push(Item::Param(p));
                        }
                    }
                }
            }
            t::Item::Typedef(td) => {
                self.typedef(td, rest)?;
                items.append(&mut self.pending);
            }
            t::Item::Import(v, _) => self.import(v, false)?,
            t::Item::Modport(v, _) => {
                for m in v {
                    let n = self.name(&m.name, m.loc);
                    self.modports.push(n);
                }
            }
            t::Item::ContAssign(c) => {
                let strength = c.strength.as_ref().and_then(super::types::drive_strength);
                let delay = match &c.delay {
                    Some(d) => Some(self.delay(d)?),
                    None => None,
                };
                for (l, r, loc) in &c.assigns {
                    let lhs = self.lvalue(l)?;
                    let rhs = self.expr(r)?;
                    items.push(Item::ContAssign { delay: delay.clone(), lhs, rhs, strength, line: loc.line });
                }
            }
            t::Item::Process(p) => {
                let kind = match p.kind {
                    t::ProcKind::Initial => ProcKind::Initial,
                    t::ProcKind::Always => ProcKind::Always,
                    t::ProcKind::AlwaysComb => ProcKind::AlwaysComb,
                    t::ProcKind::AlwaysFf => ProcKind::AlwaysFf,
                    t::ProcKind::AlwaysLatch => ProcKind::AlwaysLatch,
                    t::ProcKind::Final => ProcKind::Final,
                };
                let body = self.stmt(&p.body)?;
                items.push(Item::Process { kind, body, line: p.loc.line });
            }
            t::Item::Subroutine(s) => {
                let s = self.subroutine(s)?;
                items.push(Item::Subroutine(s));
            }
            t::Item::Generate(v, _) => {
                let refs: Vec<&t::Item> = v.iter().collect();
                self.item_list(&refs, items)?;
            }
            t::Item::GenFor(g) => {
                let init_var = self.name(&g.var, g.loc);
                let init = self.expr(&g.init)?;
                let cond = self.expr(&g.cond)?;
                let (step_var, step) = self.gen_step(&g.step)?;
                let (name, sub) = self.gen_items(&g.body)?;
                items.push(Item::GenFor { init_var, init, cond, step_var, step, name, items: sub, line: g.loc.line });
            }
            t::Item::GenIf(g) => {
                let cond = self.expr(&g.cond)?;
                let (then_name, then_items) = self.gen_items(&g.then_)?;
                let (else_name, else_items) = match &g.else_ {
                    Some(b) => self.gen_items(b)?,
                    None => (None, Vec::new()),
                };
                items.push(Item::GenIf { cond, then_items, then_name, else_items, else_name, line: g.loc.line });
            }
            t::Item::GenCase(g) => {
                let expr = self.expr(&g.expr)?;
                let mut arms = Vec::new();
                for (labels, b) in &g.arms {
                    let mut ls = Vec::new();
                    for l in labels {
                        ls.push(self.expr(l)?);
                    }
                    let (name, sub) = self.gen_items(b)?;
                    arms.push((ls, name, sub));
                }
                items.push(Item::GenCase { expr, arms, line: g.loc.line });
            }
            t::Item::GenBlock(b) => {
                let (name, sub) = self.gen_items(b)?;
                items.push(Item::GenBlock { name, items: sub, line: b.loc.line });
            }
            t::Item::Defparam(v, _) => {
                for (target, value) in v {
                    let Some(path) = self.hier_path(target)? else {
                        return self.unsupported(target.loc, "this defparam target");
                    };
                    let value = self.expr(value)?;
                    items.push(Item::Defparam { target: path, value, line: target.loc.line });
                }
            }
            t::Item::Specify(v, _) => self.specify(v, items)?,
            t::Item::Net(n) => {
                if n.user.is_some() || n.net.is_none() {
                    return self.unsupported(n.loc, "a net of a user-defined nettype");
                }
                let mut d = self.decl_head(None, n.net, false, &n.ty, n.strength.as_ref(), n.delay.as_ref(), n.loc)?;
                d.names = self.declarators(&n.decls)?;
                items.append(&mut self.pending);
                items.push(Item::Decl(d));
            }
            t::Item::Var(v) => {
                if v.const_ {
                    return self.unsupported(v.loc, "a const variable");
                }
                if v.lifetime == Some(t::Lifetime::Automatic) {
                    return self.unsupported(v.loc, "an automatic variable declaration");
                }
                if matches!(v.ty.kind, t::TypeKind::Implicit) && !v.var {
                    return self.unsupported(v.loc, "a declaration without a data type");
                }
                let mut d = self.decl_head(None, None, v.var, &v.ty, None, None, v.loc)?;
                d.names = self.declarators(&v.decls)?;
                items.append(&mut self.pending);
                items.push(Item::Decl(d));
            }
            t::Item::Port(p) => {
                let dir = self.dir(p.dir, p.loc)?;
                let mut d = self.decl_head(Some(dir), p.net, p.var, &p.ty, None, None, p.loc)?;
                d.names = self.declarators(&p.decls)?;
                items.append(&mut self.pending);
                items.push(Item::Decl(d));
            }
            t::Item::Genvar(v) => {
                let line = v.first().map(|x| x.1.line).unwrap_or(0);
                self.decl_type = None;
                let mut names = Vec::new();
                for (n, loc) in v {
                    let n = self.name(n, *loc);
                    self.declare_var(&n, &[]);
                    names.push(Declarator { name: n, dims: vec![], heap: None, init: None, line: loc.line });
                }
                let d = Decl {
                    dir: None,
                    kind: NetKind::Genvar,
                    net: NetType::Wire,
                    explicit_kind: true,
                    real_net: false,
                    strength: None,
                    delay: None,
                    signed: false,
                    range: None,
                    packed: vec![],
                    type_param: None,
                    ty: None,
                    names,
                    line,
                };
                items.append(&mut self.pending);
                items.push(Item::Decl(d));
            }
            t::Item::Gates(g) => {
                let kind = g.kind.as_str().to_string();
                let strength = g.strength.as_ref().and_then(super::types::drive_strength);
                let delay = match &g.delay {
                    Some(d) => Some(self.delay(d)?),
                    None => None,
                };
                for gi in &g.insts {
                    let inst_name = gi.name.as_ref().map(|n| self.name(n, gi.loc));
                    let dims = match gi.dims.as_slice() {
                        [] => None,
                        [d] => Some(self.dim(d, gi.loc)?),
                        _ => return self.unsupported(gi.loc, "a multi-dimensional gate array"),
                    };
                    let mut terms = Vec::new();
                    for e in &gi.terms {
                        terms.push(self.expr(e)?);
                    }
                    items.push(Item::Gate { kind: kind.clone(), inst_name, dims, terms, strength, delay: delay.clone(), line: gi.loc.line });
                }
            }
            t::Item::Instances(i) => items.push(self.instances(i)?),
            other => return self.unsupported(item_loc(other), item_what(other)),
        }
        Ok(())
    }

    /// The step of a generate loop: the variable and its new value.
    fn gen_step(&mut self, s: &t::Stmt) -> Result<(String, Expr)> {
        let (lhs, rhs) = match &s.kind {
            t::StmtKind::Assign { lhs, op, timing: None, rhs } => {
                let r = self.expr(rhs)?;
                (lhs, match super::stmt::assign_binop(*op) {
                    None => r,
                    Some(b) => {
                        let t::ExprKind::Ident(n) = &lhs.kind else { return self.unsupported(s.loc, "this generate loop step") };
                        Expr::Binary(b, Box::new(Expr::Ident(n.clone())), Box::new(r))
                    }
                })
            }
            t::StmtKind::Expr(t::Expr { kind: t::ExprKind::IncDec { inc, e, .. }, .. }) => {
                let t::ExprKind::Ident(n) = &e.kind else { return self.unsupported(s.loc, "this generate loop step") };
                let op = if *inc { BinOp::Add } else { BinOp::Sub };
                (&**e, Expr::Binary(op, Box::new(Expr::Ident(n.clone())), Box::new(Expr::Num(dec(1)))))
            }
            _ => return self.unsupported(s.loc, "this generate loop step"),
        };
        let t::ExprKind::Ident(n) = &lhs.kind else { return self.unsupported(s.loc, "this generate loop step") };
        Ok((self.name(n, lhs.loc), rhs))
    }

    fn instances(&mut self, i: &t::Instances) -> Result<Item> {
        let module = i.module.clone();
        let mut params = Vec::new();
        if let Some(ps) = &i.params {
            for p in ps {
                params.push(match p {
                    t::ParamArg::Positional(e) => Conn::Positional(Some(self.expr(e)?)),
                    t::ParamArg::Empty => Conn::Positional(None),
                    t::ParamArg::Named(n, e) => Conn::Named(
                        self.name(n, i.loc),
                        match e {
                            Some(e) => Some(self.expr(e)?),
                            None => None,
                        },
                    ),
                });
            }
        }
        let mut insts = Vec::new();
        for x in &i.insts {
            let name = match &x.name {
                Some(n) => self.name(n, x.loc),
                None => {
                    self.unnamed += 1;
                    format!("${module}${}", self.unnamed)
                }
            };
            let dims = self.dims(&x.dims, x.loc)?;
            let mut conns = Vec::new();
            for c in &x.conns {
                conns.push(match c {
                    t::PortConn::Positional(e) => Conn::Positional(match e {
                        Some(e) => Some(self.expr(e)?),
                        None => None,
                    }),
                    t::PortConn::Named(n, Some(e)) => Conn::Named(
                        n.clone(),
                        match e {
                            Some(e) => Some(self.expr(e)?),
                            None => None,
                        },
                    ),
                    t::PortConn::Named(n, None) => Conn::Named(n.clone(), Some(Expr::Ident(n.clone()))),
                    t::PortConn::Wildcard => Conn::Wildcard,
                });
            }
            insts.push(Instance { name, dims, conns, line: x.loc.line });
        }
        Ok(Item::Instances { module, params, insts, line: i.loc.line })
    }

    /// specparams and path delays are kept; conditions, edges and timing checks ignored.
    fn specify(&mut self, v: &[t::SpecifyItem], items: &mut Vec<Item>) -> Result<()> {
        for s in v {
            match s {
                t::SpecifyItem::Specparam(p) => {
                    let t::ParamKind::Value { ty, assigns } = &p.kind else {
                        return self.unsupported(p.loc, "a type specparam");
                    };
                    let pt = self.param_type(ty)?;
                    for d in assigns {
                        let p = self.param_assign(true, &pt, d)?;
                        items.push(Item::Param(p));
                    }
                }
                t::SpecifyItem::Path(p) => {
                    let mut srcs = Vec::new();
                    for e in &p.srcs {
                        spec_names(e, &mut srcs);
                    }
                    let mut dsts = Vec::new();
                    for e in &p.dsts {
                        spec_names(e, &mut dsts);
                    }
                    if p.data.is_some() {
                        dsts.truncate(1);
                    }
                    let delay = match &p.delays {
                        t::Delay::List(v) if v.len() > 3 => self.delay(&t::Delay::List(v[..3].to_vec()))?,
                        d => self.delay(d)?,
                    };
                    items.push(Item::SpecPath { srcs, dsts, delay, line: p.loc.line });
                }
                t::SpecifyItem::TimingCheck { .. } | t::SpecifyItem::Pulse { .. } => {}
            }
        }
        Ok(())
    }

    /// A function or task.
    fn subroutine(&mut self, s: &t::Subroutine) -> Result<SubroutineDecl> {
        let line = s.loc.line;
        if !s.quals.is_empty() {
            return self.unsupported(s.loc, "a qualified subroutine (class method)");
        }
        if !s.name.scopes.is_empty() {
            return self.unsupported(s.loc, "an out-of-class method body");
        }
        let Some((body_items, body_stmts)) = &s.body else {
            return self.unsupported(s.loc, "a subroutine prototype");
        };
        let is_task = s.is_task;
        let automatic = s.lifetime == Some(t::Lifetime::Automatic);
        let mut signed = false;
        let mut range = None;
        let mut ret_kind = NetKind::Reg;
        let mut ret_type = None;
        if let Some(rt) = &s.ret {
            let sv = matches!(rt.kind, t::TypeKind::Struct(_) | t::TypeKind::Enum(_) | t::TypeKind::Named(_));
            if sv {
                let t = self.data_type_req(rt)?;
                let (k, sg, r, packed) = t.decl_parts();
                range = if !packed.is_empty() { Some((super::types::minus_one(t.width()), Expr::Num(dec(0)))) } else { r };
                signed = sg;
                ret_kind = match k {
                    NetKind::Int(_) | NetKind::Integer if range.is_none() => NetKind::Integer,
                    _ => NetKind::Reg,
                };
                if ret_kind == NetKind::Integer {
                    range = None;
                }
                ret_type = Some(std::rc::Rc::new(t));
            } else {
                signed = rt.signed == Some(true);
                let width = |w: u64| Some((Expr::Num(dec(w - 1)), Expr::Num(dec(0))));
                match &rt.kind {
                    t::TypeKind::Void => ret_kind = NetKind::Int(0),
                    t::TypeKind::Implicit | t::TypeKind::Int(t::IntKw::Logic | t::IntKw::Reg | t::IntKw::Bit) => {}
                    t::TypeKind::Int(t::IntKw::Integer | t::IntKw::Int) => {
                        ret_kind = NetKind::Integer;
                        signed = rt.signed != Some(false);
                    }
                    t::TypeKind::Int(t::IntKw::Byte) => {
                        range = width(8);
                        signed = rt.signed != Some(false);
                    }
                    t::TypeKind::Int(t::IntKw::Shortint) => {
                        range = width(16);
                        signed = rt.signed != Some(false);
                    }
                    t::TypeKind::Int(t::IntKw::Longint) => {
                        range = width(64);
                        signed = rt.signed != Some(false);
                    }
                    t::TypeKind::Int(t::IntKw::Time) => range = width(64),
                    t::TypeKind::Real(t::RealKw::Real | t::RealKw::Realtime) => ret_kind = NetKind::Real,
                    t::TypeKind::String => ret_kind = NetKind::String,
                    _ => return self.unsupported(rt.loc, "this return type"),
                }
                match rt.packed.as_slice() {
                    [] => {}
                    [d] if range.is_none() => range = Some(self.dim(d, rt.loc)?),
                    _ => return self.unsupported(rt.loc, "a return type with several packed dimensions"),
                }
            }
        }
        let name = self.name(&s.name.name, s.name.loc);
        // the function's own name is its result variable
        self.decl_type = ret_type.clone();
        self.declare_var(&name, &[]);
        let mut decls = Vec::new();
        if let Some(ports) = &s.ports {
            let mut cur: Option<Decl> = None;
            for p in ports {
                if !p.dims.is_empty() && !matches!(p.dims.as_slice(), [t::Dim::Unsized | t::Dim::Queue(_)]) {
                    return self.unsupported(p.loc, "an array argument");
                }
                if p.dir.is_some() || p.ty.is_some() || p.var || cur.is_none() {
                    let dir = match p.dir {
                        Some(d) => Some(self.dir(d, p.loc)?),
                        None => Some(cur.as_ref().and_then(|c| c.dir).unwrap_or(Dir::Input)),
                    };
                    let implicit = t::DataType { kind: t::TypeKind::Implicit, signed: None, packed: vec![], loc: p.loc };
                    let mut d = self.decl_head(dir, None, p.var, p.ty.as_ref().unwrap_or(&implicit), None, None, p.loc)?;
                    if !d.explicit_kind {
                        d.kind = NetKind::Reg;
                    }
                    cur = Some(d);
                }
                let n = self.name(&p.name, p.loc);
                let (dims, heap) = self.declarator_dims(&p.dims, p.loc)?;
                self.declare_var_heap(&n, &dims, heap.as_ref());
                let init = match &p.default {
                    Some(e) => Some(self.expr(e)?),
                    None => None,
                };
                let mut d = cur.clone().unwrap();
                d.names = vec![Declarator { name: n, dims, heap, init, line: p.loc.line }];
                decls.push(d);
            }
        }
        // declarations (also non-ANSI ports) and statements, in source order
        for it in body_items {
            let (dir, net, var, ty, dl, loc) = match it {
                t::Item::Port(p) => (Some(self.dir(p.dir, p.loc)?), p.net, p.var, &p.ty, &p.decls, p.loc),
                t::Item::Var(v) => {
                    if v.const_ {
                        return self.unsupported(v.loc, "a const variable");
                    }
                    if v.lifetime.is_some() {
                        // (their initial values are not applied yet)
                        return self.unsupported(v.loc, "an explicit variable lifetime in a subroutine");
                    }
                    (None, None, v.var, &v.ty, &v.decls, v.loc)
                }
                t::Item::Param(p) => return self.err(p.loc, "parameters inside functions are not supported"),
                other => return self.unsupported(item_loc(other), &format!("{} in a subroutine", item_what(other))),
            };
            let mut d = self.decl_head(dir, net, var, ty, None, None, loc)?;
            if !d.explicit_kind {
                d.kind = NetKind::Reg;
            }
            d.names = self.declarators(dl)?;
            decls.push(d);
        }
        let mut stmts = Vec::new();
        self.in_subroutine = true;
        let r = body_stmts.iter().try_for_each(|st| {
            stmts.push(self.stmt(st)?);
            Ok(())
        });
        self.in_subroutine = false;
        r?;
        let body = if stmts.len() == 1 { stmts.pop().unwrap() } else { Stmt::Block { name: None, decls: vec![], stmts, fork: None } };
        let decls = super::merge_port_types(decls);
        Ok(SubroutineDecl { name, is_task, automatic, signed, range, ret_kind, ret_ty: ret_type, decls, body, line })
    }
}

/// The names in a specify path terminal (outside brackets).
fn spec_names(e: &t::Expr, out: &mut Vec<String>) {
    match &e.kind {
        t::ExprKind::Ident(n) => out.push(n.clone()),
        t::ExprKind::Index(a, _) | t::ExprKind::Range(a, ..) => spec_names(a, out),
        t::ExprKind::Member(a, m) => {
            spec_names(a, out);
            out.push(m.clone());
        }
        _ => {}
    }
}
