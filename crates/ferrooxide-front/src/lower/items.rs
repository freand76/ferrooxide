//! Module items: declarations, parameters, continuous assignments, processes, subroutines,
//! instances and gates, generate constructs, specify blocks.

use super::{item_loc, item_what, Lower};
use super::SvType;
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
        self.wild.skip += 1;
        self.in_generate += 1;
        let r = self.item_list(&refs, &mut items);
        self.in_generate -= 1;
        self.wild.skip -= 1;
        r?;
        // a conditional that is the whole block, without begin-end: no scope of its own
        if let (false, None, [Item::GenIf { direct, .. } | Item::GenCase { direct, .. }]) = (b.begin, &name, &mut items[..]) {
            *direct = true;
        }
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
                Ok(ExprKind::SysCall("$__delays".into(), out).into())
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
            // `export "DPI-C" function f;`: f callable from foreign code, of which there is none
            // (IEEE 1800-2017 35.7)
            t::Item::Dpi(d) if !d.import => {}
            t::Item::Directive(d, _) => self.directive(d),
            t::Item::Timeunits { unit, precision, loc } => {
                let (u, p) = self.timeunits(*unit, *precision, *loc)?;
                self.local_tu = (u.or(self.local_tu.0), p.or(self.local_tu.1));
            }
            t::Item::Param(p) => {
                // (with a parameter port list, the module's own parameters are local:
                // IEEE 1800-2017 6.20.1)
                let local = p.kw != Some(Kw::Parameter) || self.body_params_local;
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
                            let mut q = self.param_assign(local, false, &pt, d)?;
                            q.spec = p.kw == Some(Kw::Specparam);
                            items.push(Item::Param(q));
                        }
                    }
                }
            }
            t::Item::Typedef(td) => {
                self.typedef(td, rest)?;
                items.append(&mut self.pending);
            }
            t::Item::Import(v, _) => self.import(v, false)?,
            t::Item::Export(v, loc) => match &mut self.pkg_ctx {
                Some(c) => c.exports.extend(v.iter().map(|(p, n)| (p.clone(), n.clone(), *loc))),
                None => return self.err(*loc, "an export is only allowed in a package"),
            },
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
            t::Item::Class(c) => {
                self.wild.skip += 1;
                let c = self.class_decl(c, rest);
                self.wild.skip -= 1;
                let c = c?;
                items.push(Item::Class(std::rc::Rc::new(c)));
                items.append(&mut self.class_statics);
            }
            t::Item::Subroutine(s) => {
                // the body of an extern method, merged into its class
                if !s.name.scopes.is_empty() && self.extern_done.contains(&(format!("{}::{}", s.name.scopes[0].name, s.name.name), s.loc.line as usize)) {
                    return Ok(());
                }
                let s = self.subroutine_in(s, false)?;
                items.push(Item::Subroutine(s));
            }
            t::Item::Generate(v, loc) => {
                // (IEEE 1800-2017 27.3: a generate region is a module item only)
                if self.in_generate > 0 {
                    return self.err(*loc, "a generate region inside a generate region or block");
                }
                let refs: Vec<&t::Item> = v.iter().collect();
                self.in_generate += 1;
                let r = self.item_list(&refs, items);
                self.in_generate -= 1;
                r?;
            }
            t::Item::GenFor(g) => {
                let init_var = self.name(&g.var, g.loc);
                // (IEEE 1800-2017 27.4: the loop variable is a genvar, which an enclosing
                // loop does not use already)
                if !g.genvar && !self.genvars.0.contains(&init_var) {
                    return self.err(g.loc, format!("'{init_var}' is not a genvar"));
                }
                if self.genvars.1.contains(&init_var) {
                    return self.err(g.loc, format!("genvar '{init_var}' is already the variable of an enclosing generate loop"));
                }
                self.genvars.1.push(init_var.clone());
                let r = self.gen_for(g, init_var);
                self.genvars.1.pop();
                items.push(r?);
            }
            t::Item::GenIf(g) => {

                let cond = self.expr(&g.cond)?;
                let (then_name, then_items) = self.gen_items(&g.then_)?;
                let (else_name, else_items) = match &g.else_ {
                    Some(b) => self.gen_items(b)?,
                    None => (None, Vec::new()),
                };
                items.push(Item::GenIf { cond, then_items, then_name, else_items, else_name, direct: false, line: g.loc.line });
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
                items.push(Item::GenCase { expr, arms, direct: false, line: g.loc.line });
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
                // (IEEE 1800-2017 6.7.1: a net holds a 4-state integral or real value, or an
                // unpacked array or struct of them)
                if (d.ty.class.is_some() || d.ty.heap.is_some()) || d.names.iter().any(|x| x.heap.is_some()) {
                    return self.err(n.loc, "a net cannot be of a class, dynamic array, queue or associative array type");
                }
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
                let mut d = self.module_port_head(Some(dir), p.net, p.var, &p.ty, p.loc)?;
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
                    self.genvars.0.insert(n.clone());
                    self.declare_var(&n, &[]);
                    names.push(Declarator { name: n, dims: vec![], heap: None, init: None, line: loc.line });
                }
                let d = Decl::var(super::SvType::flat(NetKind::Genvar, false, vec![]), names, false, line);
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
            // (a deferred assertion outside a procedure is in an always_comb of its own,
            // IEEE 1800-2017 16.4.3)
            t::Item::Assertion(a) if matches!(a.body, t::AssertBody::Immediate { deferred: Some(_), .. }) => {
                let body = self.deferred_assertion(a, a.label.as_ref())?;
                items.push(Item::Process { kind: ProcKind::AlwaysComb, body, line: a.loc.line });
            }
            t::Item::Assertion(a) if matches!(a.body, t::AssertBody::Immediate { .. }) => {
                return self.err(a.loc, "an immediate assertion outside a procedure (only deferred ones, `#0` or `final`)");
            }
            t::Item::Assertion(a) => {
                if let Some(a) = self.conc_assert(a)? {
                    items.push(Item::Assert(Box::new(a)));
                }
            }
            t::Item::ElabTask { name, args, loc } => {
                let mut v = Vec::new();
                for a in args {
                    v.push(match a {
                        t::Arg::Pos(None) => None,
                        t::Arg::Pos(Some(x)) => Some(self.arg(x)?),
                        t::Arg::Named(..) => return self.unsupported(*loc, "a named argument"),
                    });
                }
                if !matches!(name.as_str(), "$fatal" | "$error" | "$warning" | "$info") {
                    return self.err(*loc, format!("'{name}' is not an elaboration system task"));
                }
                items.push(Item::ElabTask { name: name.clone(), args: v, line: loc.line });
            }
            t::Item::Property(_) | t::Item::Sequence(_) | t::Item::Clocking(_) | t::Item::ClockingRef { .. } | t::Item::DefaultDisable(..) => {
                self.sva_item(it)?;
            }
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
                        ExprKind::Binary(b, Box::new(ExprKind::Ident(n.clone()).into()), Box::new(r)).into()
                    }
                })
            }
            t::StmtKind::Expr(t::Expr { kind: t::ExprKind::IncDec { inc, e, .. }, .. }) => {
                let t::ExprKind::Ident(n) = &e.kind else { return self.unsupported(s.loc, "this generate loop step") };
                let op = if *inc { BinOp::Add } else { BinOp::Sub };
                (&**e, ExprKind::Binary(op, Box::new(ExprKind::Ident(n.clone()).into()), Box::new(ExprKind::Num(dec(1)).into())).into())
            }
            _ => return self.unsupported(s.loc, "this generate loop step"),
        };
        let t::ExprKind::Ident(n) = &lhs.kind else { return self.unsupported(s.loc, "this generate loop step") };
        Ok((self.name(n, lhs.loc), rhs))
    }

    fn gen_for(&mut self, g: &t::GenFor, init_var: String) -> Result<Item> {
        let init = self.expr(&g.init)?;
        let cond = self.expr(&g.cond)?;
        let (step_var, step) = self.gen_step(&g.step)?;
        let (name, items) = self.gen_items(&g.body)?;
        Ok(Item::GenFor { init_var, init, cond, step_var, step, name, items, line: g.loc.line })
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
            // (IEEE 1800-2017 23.3.2: by order or by name, not both)
            let by_order = x.conns.iter().any(|c| matches!(c, t::PortConn::Positional(_)));
            if by_order && x.conns.iter().any(|c| !matches!(c, t::PortConn::Positional(_))) {
                return self.err(x.loc, "port connections by order and by name are mixed");
            }
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
                    t::PortConn::Named(n, None) => Conn::Named(n.clone(), Some(ExprKind::Ident(n.clone()).into())),
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
                        let mut p = self.param_assign(true, false, &pt, d)?;
                        p.spec = true;
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
                    let whole = |v: &[t::Expr]| matches!(v, [t::Expr { kind: t::ExprKind::Ident(_), .. }]);
                    let parallel = !p.full && p.data.is_none() && whole(&p.srcs) && whole(&p.dsts);
                    items.push(Item::SpecPath { srcs, dsts, delay, parallel, line: p.loc.line });
                }
                t::SpecifyItem::TimingCheck { .. } | t::SpecifyItem::Pulse { .. } => {}
            }
        }
        Ok(())
    }

    /// A function or task.
    /// A function or task; `method`: of a class (its qualifiers are the class's business).
    pub(crate) fn subroutine_in(&mut self, s: &t::Subroutine, method: bool) -> Result<SubroutineDecl> {
        let line = s.loc.line;
        if !s.quals.is_empty() && !method {
            return self.unsupported(s.loc, "a qualified subroutine (class method)");
        }
        if !s.name.scopes.is_empty() {
            return self.unsupported(s.loc, "an out-of-class method body");
        }
        let Some((body_items, body_stmts)) = &s.body else {
            return self.unsupported(s.loc, "a subroutine prototype");
        };
        let is_task = s.is_task;
        // without its own lifetime: the module's or package's default (IEEE 1800-2017 6.21)
        let automatic = match s.lifetime {
            Some(l) => l == t::Lifetime::Automatic,
            None => self.default_automatic,
        };
        // the return type, and the type the lowering knows the result variable by (none for a
        // builtin type with at most one range)
        let mut ret_ty = SvType::vector(if is_task { NetKind::Int(0) } else { NetKind::Reg }, false, vec![]);
        let mut ret_type = None;
        if let Some(rt) = &s.ret {
            let sv = matches!(rt.kind, t::TypeKind::Struct(_) | t::TypeKind::Enum(_) | t::TypeKind::Named(_));
            if sv {
                ret_ty = self.data_type_req(rt)?;
                ret_type = Some(std::rc::Rc::new(ret_ty.clone()));
            } else {
                let signed = rt.signed == Some(true);
                let int = |k: NetKind| SvType::flat(k, rt.signed != Some(false), vec![]);
                ret_ty = match &rt.kind {
                    t::TypeKind::Void => SvType::vector(NetKind::Int(0), false, vec![]),
                    t::TypeKind::Implicit | t::TypeKind::Int(t::IntKw::Logic | t::IntKw::Reg) => SvType::vector(NetKind::Reg, signed, vec![]),
                    t::TypeKind::Int(t::IntKw::Bit) => SvType::vector(NetKind::Bit, signed, vec![]),
                    t::TypeKind::Int(t::IntKw::Integer) => int(NetKind::Integer),
                    t::TypeKind::Int(t::IntKw::Int) => int(NetKind::Int(32)),
                    t::TypeKind::Int(t::IntKw::Byte) => int(NetKind::Int(8)),
                    t::TypeKind::Int(t::IntKw::Shortint) => int(NetKind::Int(16)),
                    t::TypeKind::Int(t::IntKw::Longint) => int(NetKind::Int(64)),
                    t::TypeKind::Int(t::IntKw::Time) => SvType::flat(NetKind::Time, signed, vec![]),
                    t::TypeKind::Real(_) => SvType::vector(NetKind::Real, signed, vec![]),
                    t::TypeKind::String => SvType::vector(NetKind::String, signed, vec![]),
                    // a chandle: a 64-bit 2-state value (see `decl_head`)
                    t::TypeKind::Chandle => SvType::vector(NetKind::Bit, signed, vec![(super::types::num(63), super::types::num(0))]),
                    _ => return self.unsupported(rt.loc, "this return type"),
                };
                if !rt.packed.is_empty() {
                    if !ret_ty.dims.is_empty() || !matches!(ret_ty.kind, NetKind::Reg | NetKind::Bit) {
                        return self.err(rt.loc, "packed dimensions after a return type that has a width");
                    }
                    ret_ty.dims = self.dims(&rt.packed, rt.loc)?;
                    if rt.packed.len() > 1 {
                        ret_type = Some(std::rc::Rc::new(ret_ty.clone()));
                    }
                }
            }
        }
        let name = if method && s.name.name == "new" { s.name.name.clone() } else { self.name(&s.name.name, s.name.loc) };
        // the function's own name is its result variable
        self.decl_type = ret_type.clone();
        self.declare_var(&name, &[]);
        let mut decls = Vec::new();
        // `ref` ports: passed in and out (`Inout`), checked by the compiler; `const ref`: in
        let mut refs = Vec::new();
        if let Some(ports) = &s.ports {
            let mut cur: Option<Decl> = None;
            let mut cur_ref = false;
            for p in ports {
                if p.dir.is_some() || p.ty.is_some() || p.var || cur.is_none() {
                    let dir = match p.dir {
                        Some(t::Dir::Ref) => {
                            cur_ref = true;
                            Some(Dir::Inout)
                        }
                        Some(t::Dir::ConstRef) => {
                            cur_ref = false;
                            Some(Dir::Input)
                        }
                        Some(d) => {
                            cur_ref = false;
                            Some(self.dir(d, p.loc)?)
                        }
                        None => Some(cur.as_ref().and_then(|c| c.dir).unwrap_or(Dir::Input)),
                    };
                    let implicit = t::DataType { kind: t::TypeKind::Implicit, signed: None, packed: vec![], loc: p.loc };
                    let mut d = self.decl_head(dir, None, p.var, p.ty.as_ref().unwrap_or(&implicit), None, None, p.loc)?;
                    if !d.explicit_kind {
                        d.set_kind(NetKind::Reg);
                    }
                    cur = Some(d);
                }
                let n = self.name(&p.name, p.loc);
                if cur_ref {
                    refs.push(n.clone());
                }
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
        let mut params = Vec::new();
        for it in body_items {
            let (dir, net, var, ty, dl, loc) = match it {
                t::Item::Port(p) => (Some(self.dir(p.dir, p.loc)?), p.net, p.var, &p.ty, &p.decls, p.loc),
                t::Item::Var(v) => {
                    if v.const_ {
                        return self.unsupported(v.loc, "a const variable");
                    }
                    match v.lifetime {
                        Some(t::Lifetime::Static) if automatic => {
                            return self.unsupported(v.loc, "a static variable in an automatic subroutine")
                        }
                        _ => {}
                    }
                    (None, None, v.var, &v.ty, &v.decls, v.loc)
                }
                t::Item::Param(_) => {
                    params.extend(self.local_params(it)?);
                    continue;
                }
                other => return self.unsupported(item_loc(other), &format!("{} in a subroutine", item_what(other))),
            };
            let mut d = self.decl_head(dir, net, var, ty, None, None, loc)?;
            if !d.explicit_kind {
                d.set_kind(NetKind::Reg);
            }
            d.names = self.declarators(dl)?;
            // (`automatic` in a static subroutine: initialized at each call)
            d.automatic = !automatic && matches!(it, t::Item::Var(v) if v.lifetime == Some(t::Lifetime::Automatic));
            decls.push(d);
        }
        let mut stmts = Vec::new();
        self.in_subroutine = true;
        let saved_fn = std::mem::replace(&mut self.in_function, !is_task);
        let nlocals = self.wild.locals.len();
        self.wild.locals.push(name.clone());
        self.wild.locals.extend(decls.iter().flat_map(|d| d.names.iter().map(|n| n.name.clone())));
        self.wild.locals.extend(params.iter().map(|p: &ParamDecl| p.name.clone()));
        let r = body_stmts.iter().try_for_each(|st| {
            stmts.push(self.stmt(st)?);
            Ok(())
        });
        self.wild.locals.truncate(nlocals);
        self.in_function = saved_fn;
        self.in_subroutine = false;
        r?;
        let body = if stmts.len() == 1 { stmts.pop().unwrap() } else { Stmt::Block { name: None, decls: vec![], imports: vec![], params: vec![], stmts, fork: None } };
        let written = decls.clone();
        let (decls, scalar_vec_ports) = super::merge_port_types(decls);
        let decls_written = if decls.len() != written.len() || decls.iter().zip(&written).any(|(a, b)| a.names.len() != b.names.len()) { written } else { Vec::new() };
        Ok(SubroutineDecl { name, is_task, automatic, ret_ty: std::rc::Rc::new(ret_ty), decls, refs, scalar_vec_ports, decls_written, params, body, line })
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
