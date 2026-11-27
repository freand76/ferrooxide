//! Virtual interfaces (IEEE 1800-2017 25.9): a handle of an interface instance.
//!
//! A `virtual X` value is a handle of the pseudo class `virtual X` (one per interface X,
//! without properties or methods), so that assignments, `null`, arrays of handles and type
//! parameters treat it as a class handle. The handle of an instance is its scope + 1 (0:
//! `null`). An access through a handle (`vif.sig`, `vif.t()`) chooses between the
//! instances of X: a statement naming one runs as `if (vif == I1) S1 else ... else Sn`, Sk
//! naming instance k's signals directly; a loop or `wait` condition chooses each time it
//! is evaluated (`vif == I1 ? c1 : ...`). Through `null` the last instance is reached.

use crate::types::Type;
use crate::*;

impl<'a> Compiler<'a> {
    /// The pseudo class `virtual X` of each interface X.
    pub(crate) fn declare_vifs(&mut self, modules: &'a [Module]) {
        fn walk<'b>(m: &'b Module, out: &mut Vec<&'b Module>) {
            if m.interface {
                out.push(m);
            }
            m.nested.iter().for_each(|n| walk(n, out));
        }
        let mut ifs = Vec::new();
        modules.iter().for_each(|m| walk(m, &mut ifs));
        for m in ifs {
            if self.classes.vifs.contains_key(&m.name) {
                continue;
            }
            let name = format!("virtual {}", m.name);
            let decl: &'a ClassDecl = Box::leak(Box::new(ClassDecl {
                name: name.clone(),
                params: vec![],
                specs: vec![],
                base: None,
                base_args: None,
                interface: false,
                interfaces: vec![],
                props: vec![],
                static_props: vec![],
                types: vec![],
                consts: vec![],
                methods: vec![],
                virtuals: vec![],
                pures: vec![],
                statics: vec![],
                abstract_: true,
                hidden: vec![],
                rand: vec![],
                constraints: vec![],
                line: m.line,
            }));
            let scope = self.new_scope(name.clone(), None, m.file.clone());
            let id = self.reserve_class(decl, &name, scope);
            self.classes.list[id as usize].scope = scope;
            self.classes.of_scope.insert(scope, id);
            self.classes.vifs.insert(m.name.clone(), id);
        }
    }

    /// The interface whose instances handles of class `c` refer to.
    pub(crate) fn vif_iface(&self, c: u32) -> Option<&str> {
        let n = self.classes.list.get(c as usize)?.name.strip_prefix("virtual ")?;
        (self.classes.vifs.get(n) == Some(&c)).then_some(n)
    }

    /// The instance scopes of interface `iface`, in elaboration order.
    fn vif_instances(&self, iface: &str) -> Vec<usize> {
        let mut v: Vec<usize> = self.hier.scope_module.iter().filter(|(_, m)| m.interface && m.name == iface).map(|(&s, _)| s).collect();
        v.sort_unstable();
        v
    }

    /// The interface instance an expression names (`bus`, `top.bus`, `ifs[2]`) where a value
    /// is expected: (its pseudo class, its scope).
    pub(crate) fn iface_instance(&self, e: &Expr, sid: usize) -> Option<(u32, usize)> {
        if self.classes.vifs.is_empty() {
            return None;
        }
        let parts: Vec<String> = match &e.kind {
            ExprKind::Ident(n) if self.lookup(n, sid).is_none() => vec![n.clone()],
            ExprKind::Hier(p) if self.lookup_hier(p, sid).is_none() => p.clone(),
            ExprKind::Index(b, i) => {
                let mut p = match &b.kind {
                    ExprKind::Ident(n) if self.lookup(n, sid).is_none() => vec![n.clone()],
                    ExprKind::Hier(p) if self.lookup_hier(p, sid).is_none() => p.clone(),
                    _ => return None,
                };
                let k = self.const_value(i, sid)?;
                let last = p.pop()?;
                p.push(format!("{last}[{k}]"));
                p
            }
            _ => return None,
        };
        let s = self.hier_scope(&parts, sid)?;
        let m = self.hier.scope_module.get(&s)?;
        if !m.interface {
            return None;
        }
        Some((*self.classes.vifs.get(&m.name)?, s))
    }

    /// The handle value of an interface instance (`iface_instance`).
    pub(crate) fn iface_handle(&mut self, s: usize) -> u32 {
        self.const_slot(&Const::from_i64(s as i64 + 1, 64, false))
    }

    /// An access through a virtual interface handle in `e`, the outermost first: the handle
    /// expression and its interface. (An instance named directly, `bus.x`, is no handle.)
    fn vif_access(&mut self, e: &Expr, sid: usize) -> Option<(Expr, String)> {
        fn bases(e: &mut Expr, out: &mut Vec<Expr>) {
            if let ExprKind::Member(b, _) | ExprKind::MethodCall(b, _, _) = &e.kind {
                if matches!(&b.kind, ExprKind::Ident(_) | ExprKind::Hier(_) | ExprKind::Member(..) | ExprKind::Index(..) | ExprKind::Call(..) | ExprKind::MethodCall(..)) {
                    out.push((**b).clone());
                }
            }
            e.for_each_child_mut(&mut |c| bases(c, out));
        }
        let mut cands = Vec::new();
        bases(&mut e.clone(), &mut cands);
        cands.into_iter().find_map(|b| {
            if self.iface_instance(&b, sid).is_some() {
                return None;
            }
            let Ok(Type::Class(c)) = self.type_of(&b, sid) else { return None };
            let i = self.vif_iface(c)?.to_string();
            Some((b, i))
        })
    }

    /// `e` with each access through handle `h` reaching instance `s` directly.
    fn vif_subst(e: &mut Expr, key: &str, s: usize) {
        let hit = match &e.kind {
            ExprKind::Member(b, _) | ExprKind::MethodCall(b, _, _) => format!("{b:?}") == key,
            _ => false,
        };
        if hit {
            let base = format!("$scope#{s}");
            let line = e.line;
            match std::mem::replace(&mut e.kind, ExprKind::Null) {
                ExprKind::Member(_, m) => e.kind = ExprKind::Hier(vec![base, m]),
                ExprKind::MethodCall(_, m, mut args) => {
                    args.iter_mut().for_each(|a| Self::vif_subst(a, key, s));
                    e.kind = ExprKind::Call(format!("{base}.{m}"), args);
                }
                _ => unreachable!(),
            }
            e.line = line;
            return;
        }
        e.for_each_child_mut(&mut |c| Self::vif_subst(c, key, s));
    }

    /// `h == handle of s`.
    fn vif_test(h: &Expr, s: usize) -> Expr {
        let k: Expr = ExprKind::Num(ferrooxide_front::ast::Number {
            size: Some(64),
            signed: false,
            base: 10,
            based: true,
            digits: (s + 1).to_string(),
            fill: false,
        })
        .into();
        ExprKind::Binary(BinOp::CaseEq, Box::new(h.clone()), Box::new(k)).into()
    }

    /// The instances of `iface` (an error if it has none).
    fn vif_targets(&self, iface: &str) -> R<Vec<usize>> {
        let v = self.vif_instances(iface);
        if v.is_empty() {
            return self.err(format!("interface '{iface}' has no instance for a virtual interface to refer to"));
        }
        Ok(v)
    }

    /// Expression `e` with its accesses through virtual interface handles resolved, each
    /// choosing the instance when evaluated; None if it has none.
    pub(crate) fn vif_expr(&mut self, e: &Expr, sid: usize) -> R<Option<Expr>> {
        let Some((h, iface)) = self.vif_access(e, sid) else { return Ok(None) };
        let key = format!("{h:?}");
        let insts = self.vif_targets(&iface)?;
        let mut out: Option<Expr> = None;
        for &s in insts.iter().rev() {
            let mut x = e.clone();
            Self::vif_subst(&mut x, &key, s);
            out = Some(match out {
                None => x,
                Some(rest) => {
                    let mut c: Expr = ExprKind::Cond(Box::new(Self::vif_test(&h, s)), Box::new(x), Box::new(rest)).into();
                    c.line = e.line;
                    c
                }
            });
        }
        // (further handles in it)
        let out = out.unwrap();
        Ok(Some(self.vif_expr(&out, sid)?.unwrap_or(out)))
    }

    /// Statement `st` (not its statements) with its first handle accessed resolved: a choice
    /// between the interface's instances. None if it accesses none.
    pub(crate) fn vif_stmt(&mut self, st: &Stmt, sid: usize) -> R<Option<Stmt>> {
        if self.classes.vifs.is_empty() {
            return Ok(None);
        }
        // conditions evaluated again and again choose each time
        let cond = |c: &mut Self, e: &Expr| c.vif_expr(e, sid);
        match st {
            Stmt::While { cond: e, body, line } => {
                return Ok(cond(self, e)?.map(|e| Stmt::While { cond: e, body: body.clone(), line: *line }));
            }
            Stmt::DoWhile { cond: e, body, line } => {
                return Ok(cond(self, e)?.map(|e| Stmt::DoWhile { cond: e, body: body.clone(), line: *line }));
            }
            Stmt::For { init, cond: e, step, body, line } => {
                return Ok(cond(self, e)?.map(|e| Stmt::For { init: init.clone(), cond: e, step: step.clone(), body: body.clone(), line: *line }));
            }
            Stmt::Wait { cond: e, body, line } => {
                return Ok(cond(self, e)?.map(|e| Stmt::Wait { cond: e, body: body.clone(), line: *line }));
            }
            // `@(vif.clk) s`: the wait chooses, then `s` runs
            Stmt::Timed { timing, body, line } if !matches!(**body, Stmt::Null) => {
                let w = Stmt::Timed { timing: timing.clone(), body: Box::new(Stmt::Null), line: *line };
                if self.vif_stmt(&w, sid)?.is_none() {
                    return Ok(None);
                }
                return Ok(Some(Stmt::Block { name: None, decls: vec![], imports: vec![], params: vec![], stmts: vec![w, (**body).clone()], fork: None }));
            }
            _ => {}
        }
        let exprs = Self::own_exprs(st);
        if exprs.is_empty() {
            return Ok(None);
        }
        // `vif.t(args);`: a task of the interface
        if let Stmt::MethodCall { obj, name, args, line } = st {
            if let (Ok(Type::Class(c)), None) = (self.type_of(obj, sid), self.iface_instance(obj, sid)) {
                if let Some(i) = self.vif_iface(c).map(str::to_string) {
                    let call: Expr = ExprKind::MethodCall(Box::new(obj.clone()), name.clone(), args.clone()).into();
                    let insts = self.vif_targets(&i)?;
                    let key = format!("{obj:?}");
                    let mut out: Option<Stmt> = None;
                    for &s in insts.iter().rev() {
                        let mut x = call.clone();
                        Self::vif_subst(&mut x, &key, s);
                        let ExprKind::Call(n, a) = x.kind else { unreachable!() };
                        let x = Stmt::TaskCall { name: n, args: a, line: *line };
                        out = Some(match out {
                            None => x,
                            Some(rest) => Stmt::If { cond: Self::vif_test(obj, s), then_s: Box::new(x), else_s: Some(Box::new(rest)), line: *line },
                        });
                    }
                    return Ok(out);
                }
            }
        }
        let hit = exprs.into_iter().find_map(|e| self.vif_access(e, sid));
        let Some((h, iface)) = hit else { return Ok(None) };
        let key = format!("{h:?}");
        let insts = self.vif_targets(&iface)?;
        let line = self.line;
        let mut out: Option<Stmt> = None;
        for &s in insts.iter().rev() {
            let mut x = st.clone();
            Self::stmt_exprs(&mut x, &mut |e| Self::vif_subst(e, &key, s));
            out = Some(match out {
                None => x,
                Some(rest) => Stmt::If { cond: Self::vif_test(&h, s), then_s: Box::new(x), else_s: Some(Box::new(rest)), line },
            });
        }
        Ok(out)
    }

    /// The expressions of statement `st` itself (not of its statements).
    fn own_exprs(st: &Stmt) -> Vec<&Expr> {
        let mut v: Vec<&Expr> = Vec::new();
        fn timing(t: &Timing) -> Vec<&Expr> {
            let (n, ev) = match t {
                Timing::Delay(e) => return vec![e],
                Timing::Event(ev) => (None, ev),
                Timing::Repeat(n, ev) => (Some(n), ev),
            };
            let mut out: Vec<&Expr> = n.into_iter().collect();
            if let EventCtl::List(l) = ev {
                out.extend(l.iter().map(|(_, e)| e));
            }
            out
        }
        match st {
            Stmt::Assign { lhs, rhs, timing: t, .. } => {
                v.extend([lhs, rhs]);
                if let Some(t) = t {
                    v.extend(timing(t));
                }
            }
            Stmt::If { cond, .. } | Stmt::Wait { cond, .. } | Stmt::While { cond, .. } | Stmt::DoWhile { cond, .. } | Stmt::For { cond, .. } | Stmt::Deferred { cond, .. } => v.push(cond),
            Stmt::Case { expr, arms, .. } => {
                v.push(expr);
                v.extend(arms.iter().flat_map(|(l, _)| l.iter()));
            }
            Stmt::Repeat { count, .. } => v.push(count),
            Stmt::Timed { timing: t, .. } => v.extend(timing(t)),
            Stmt::SysTask { args, .. } => v.extend(args.iter().flatten()),
            Stmt::TaskCall { args, .. } | Stmt::StaticCall { args, .. } => v.extend(args.iter()),
            Stmt::MethodCall { obj, args, .. } => {
                v.push(obj);
                v.extend(args.iter());
            }
            Stmt::Force { lhs, rhs, .. } => {
                v.push(lhs);
                v.extend(rhs.iter());
            }
            Stmt::Trigger { target, .. } => v.push(target),
            Stmt::Return { value: Some(e), .. } | Stmt::Discard { e, .. } | Stmt::Foreach { array: e, .. } => v.push(e),
            _ => {}
        }
        v
    }

    /// Calls `f` for each expression of statement `st` itself (not of its statements).
    fn stmt_exprs(st: &mut Stmt, f: &mut dyn FnMut(&mut Expr)) {
        let timing = |t: &mut Timing, f: &mut dyn FnMut(&mut Expr)| match t {
            Timing::Delay(e) => f(e),
            Timing::Event(ev) => {
                if let EventCtl::List(v) = ev {
                    v.iter_mut().for_each(|(_, e)| f(e));
                }
            }
            Timing::Repeat(n, ev) => {
                f(n);
                if let EventCtl::List(v) = ev {
                    v.iter_mut().for_each(|(_, e)| f(e));
                }
            }
        };
        match st {
            Stmt::Assign { lhs, rhs, timing: t, .. } => {
                f(lhs);
                f(rhs);
                if let Some(t) = t {
                    timing(t, f);
                }
            }
            Stmt::If { cond, .. } | Stmt::Wait { cond, .. } | Stmt::While { cond, .. } | Stmt::DoWhile { cond, .. } | Stmt::For { cond, .. } => f(cond),
            Stmt::Deferred { cond, .. } => f(cond),
            Stmt::Case { expr, arms, .. } => {
                f(expr);
                arms.iter_mut().for_each(|(v, _)| v.iter_mut().for_each(&mut *f));
            }
            Stmt::Repeat { count, .. } => f(count),
            Stmt::Timed { timing: t, .. } => timing(t, f),
            Stmt::SysTask { args, .. } => args.iter_mut().flatten().for_each(f),
            Stmt::TaskCall { args, .. } | Stmt::StaticCall { args, .. } => args.iter_mut().for_each(f),
            Stmt::MethodCall { obj, args, .. } => {
                f(obj);
                args.iter_mut().for_each(f);
            }
            Stmt::Force { lhs, rhs, .. } => {
                f(lhs);
                if let Some(r) = rhs {
                    f(r);
                }
            }
            Stmt::Trigger { target, .. } => f(target),
            Stmt::Return { value: Some(v), .. } => f(v),
            Stmt::Discard { e, .. } => f(e),
            Stmt::Foreach { array, .. } => f(array),
            _ => {}
        }
    }
}
