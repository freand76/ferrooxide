//! Instances: a module instantiated with its parameters and port connections.

use crate::*;

impl<'a> Compiler<'a> {
    /// A port connected to a constant-index array element of the same width becomes an alias of
    /// the element (no copy process). Returns the alias signal.
    pub(crate) fn elem_port_alias(&mut self, conn: &Expr, ps: usize, width: u32, name: &str) -> Option<u32> {
        let Expr::Index(a, i) = conn else { return None };
        let arr = self.array_of(a, ps)?;
        let e = self.const_value(i, ps)? - self.sig(arr).arr_lo;
        let info = self.sig(arr).clone();
        if info.width != width || e < 0 || e >= info.depth as i64 {
            return None;
        }
        let e = e as u32;
        if let Some(&s) = self.nets.elem_alias.get(&(arr, e)) {
            return Some(s);
        }
        let ty = self.sig_type(arr);
        let id = self.push_signal(SignalInfo {
            name: name.to_string(),
            slot: info.slot + e * rt::words(width),
            width,
            signed: info.signed,
            depth: 0,
            msb: info.msb,
            lsb: info.lsb,
            arr_lo: 0,
            kind: info.kind,
            alias_of: Some((arr, e)),
            real: info.real,
            forced: false,
            string: info.string,
            event: info.event,
        }, ty);
        self.nets.elem_alias.insert((arr, e), id);
        Some(id)
    }

    /// The order in which processes start at time 0, as in the reference: top-level modules
    /// by name; in a scope first its generate scopes, then its instances (in source order,
    /// each with everything below it), then its own processes in source order.
    pub(crate) fn start_order(&mut self) {
        let n = self.scopes.len();
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (s, sc) in self.scopes.iter().enumerate() {
            let mut v: Vec<usize> = sc.children.values().copied().filter(|&c| c != s).collect();
            v.sort_unstable();
            v.dedup();
            // generate scopes (their parent is this scope) before instances
            v.sort_by_key(|&c| (self.scopes[c].parent != Some(s), c));
            children[s] = v;
        }
        let mut rank = vec![usize::MAX; n];
        let mut next = 0;
        let mut tops: Vec<(&String, &usize)> = self.hier.tops.iter().collect();
        tops.sort();
        fn visit(s: usize, children: &[Vec<usize>], rank: &mut [usize], next: &mut usize) {
            if rank[s] != usize::MAX {
                return;
            }
            rank[s] = usize::MAX - 1;
            for &c in &children[s] {
                visit(c, children, rank, next);
            }
            rank[s] = *next;
            *next += 1;
        }
        for (_, &t) in tops {
            visit(t, &children, &mut rank, &mut next);
        }
        let key = |p: usize| -> usize {
            match self.code.proc_scope.get(p) {
                Some(&s) if s < n => rank[s].saturating_add(1),
                // continuous assignments, port connections: first, in creation order
                Some(&s) if s == usize::MAX - 1 => 0,
                _ => usize::MAX,
            }
        };
        let mut order: Vec<u32> = (0..self.prog.procs.len() as u32).collect();
        order.sort_by_key(|&p| (key(p as usize), p));
        self.prog.start_order = order;
    }

    pub(crate) fn instantiate(
        &mut self,
        m: &'a Module,
        path: String,
        overrides: &[(Option<String>, PVal)],
        conns: Option<(&'a [Conn], usize)>,
    ) -> R<usize> {
        let saved_file = std::mem::replace(&mut self.file, m.file.clone());
        // a nested module looks up what it does not declare in the enclosing instance
        let outer = if m.is_nested { conns.map(|c| c.1) } else { None };
        let sid = self.new_scope(path.clone(), outer, m.file.clone());
        self.scopes[sid].inst_parent = conns.map(|c| c.1);
        self.hier.scope_module.insert(sid, m);
        self.scopes[sid].imports = m.imports.iter().filter_map(|n| self.hier.pkg_scopes.get(n).copied()).collect();
        self.scopes[sid].tunit = m.timescale.map(|t| t.0).unwrap_or(0);
        self.scopes[sid].tprec = m.timescale.map(|t| t.1).unwrap_or(0);
        self.scopes[sid].nettype = m.default_nettype.clone();
        self.scopes[sid].uncdrive = m.unconnected_drive;
        for it in &m.items {
            if let Item::Subroutine(f) = it {
                self.scopes[sid].funcs.insert(f.name.clone(), f);
            }
        }
        self.eval_params(m, overrides, sid)?;
        // port connections by name
        let mut port_conns: HashMap<String, Option<Expr>> = HashMap::new();
        if let Some((cs, _)) = conns {
            let mut idx = 0;
            for c in cs {
                match c {
                    Conn::Positional(e) => {
                        if let Some(pn) = m.ports.get(idx) {
                            port_conns.insert(pn.clone(), e.clone());
                        } else if e.is_some() {
                            return self.err(format!("too many port connections for module '{}'", m.name));
                        }
                        idx += 1;
                    }
                    Conn::Named(n, e) => {
                        if !m.ports.contains(n) {
                            return self.err(format!("module '{}' has no port '{n}'", m.name));
                        }
                        port_conns.insert(n.clone(), e.clone());
                    }
                    Conn::Wildcard => {
                        for p in &m.ports {
                            port_conns.entry(p.clone()).or_insert_with(|| Some(Expr::Ident(p.clone())));
                        }
                    }
                }
            }
        }
        let parent_scope = conns.map(|c| c.1);
        let inst_line = self.line;
        // interface ports: the port's name stands for the connected interface instance
        for (pn, iface, dims, modport) in &m.iface_ports {
            if let (Some(mp), Some(im)) = (modport, self.hier.mods.get(iface.as_str())) {
                if im.interface && !im.modports.contains(mp) {
                    return self.err(format!("interface '{iface}' has no modport '{mp}' (port '{pn}' of '{}')", m.name));
                }
            }
            // the connected instance must be one of the port's interface
            let check = |c: &Self, t: usize| -> R<()> {
                match c.hier.scope_module.get(&t) {
                    Some(tm) if tm.interface && tm.name != *iface => {
                        c.err(format!("interface port '{pn}' of '{}' is connected to an instance of '{}', not '{iface}'", m.name, tm.name))
                    }
                    _ => Ok(()),
                }
            };
            if let Some((pl, pr)) = dims {
                // an array port: the element k above the lowest index is element k above the
                // lowest index of the connected array, as the reference does (ICARUS_NOTES 10)
                let (pl, pr) = (self.const_int(pl, sid)?, self.const_int(pr, sid)?);
                let actual = match (port_conns.get(pn), parent_scope) {
                    (Some(Some(e)), Some(ps)) => {
                        let parts: Vec<String> = match e {
                            Expr::Ident(n) => vec![n.clone()],
                            Expr::Hier(p) => p.clone(),
                            _ => vec![],
                        };
                        match parts.split_last() {
                            Some((last, pre)) if pre.is_empty() => {
                                let mut x = Some(ps);
                                let mut found = None;
                                while let Some(s) = x {
                                    if let Some(&lr) = self.hier.inst_arrays.get(&(s, last.clone())) {
                                        found = Some((s, last.clone(), lr));
                                        break;
                                    }
                                    x = self.scopes[s].parent;
                                }
                                found
                            }
                            Some((last, pre)) => self
                                .hier_scope(pre, ps)
                                .and_then(|s| self.hier.inst_arrays.get(&(s, last.clone())).map(|&lr| (s, last.clone(), lr))),
                            None => None,
                        }
                    }
                    _ => None,
                };
                let Some((as_, an, (al, ar))) = actual else {
                    return self.err(format!("interface port '{pn}' of '{}' must be connected to an array of '{iface}' instances", m.name));
                };
                let n = (pl - pr).abs() + 1;
                if (al - ar).abs() + 1 != n {
                    return self.err(format!(
                        "interface port '{pn}' of '{}' has {n} elements, the connected '{an}' has {}",
                        m.name,
                        (al - ar).abs() + 1
                    ));
                }
                for k in 0..n {
                    let (pi, ai) = (pl.min(pr) + k, al.min(ar) + k);
                    let Some(&t) = self.scopes[as_].children.get(&format!("{an}[{ai}]")) else {
                        return self.err(format!("'{an}[{ai}]' is not an interface instance"));
                    };
                    check(self, t)?;
                    self.scopes[sid].children.insert(format!("{pn}[{pi}]"), t);
                }
                self.hier.inst_arrays.insert((sid, pn.clone()), (pl, pr));
                continue;
            }
            let target = match (port_conns.get(pn), parent_scope) {
                (Some(Some(e)), Some(ps)) => {
                    let parts: Vec<String> = match e {
                        Expr::Ident(n) => vec![n.clone()],
                        Expr::Hier(p) => p.clone(),
                        // an element of an array of interface instances: `ifs[2]` (constant)
                        Expr::Index(b, i) if matches!(**b, Expr::Ident(_) | Expr::Hier(_)) => {
                            let k = self.const_int(i, ps)?;
                            let mut p = match &**b {
                                Expr::Ident(n) => vec![n.clone()],
                                Expr::Hier(p) => p.clone(),
                                _ => unreachable!(),
                            };
                            let last = p.pop().unwrap();
                            p.push(format!("{last}[{k}]"));
                            p
                        }
                        _ => vec![],
                    };
                    // `bus.master`: a modport of the instance
                    self.hier_scope(&parts, ps).or_else(|| {
                        (parts.len() > 1).then(|| self.hier_scope(&parts[..parts.len() - 1], ps)).flatten()
                    })
                }
                _ => None,
            };
            let Some(target) = target else {
                return self.err(format!("interface port '{pn}' of '{}' must be connected to an instance of '{iface}'", m.name));
            };
            check(self, target)?;
            self.scopes[sid].children.insert(pn.clone(), target);
        }
        self.elab_items(&m.items, sid, true, &port_conns, parent_scope)?;
        // nested modules without ports that are not instantiated explicitly: one instance
        // each, named after the module (IEEE 1800-2017 23.4)
        for n in &m.nested {
            if n.ports.is_empty() && n.iface_ports.is_empty() && !Self::instantiates(&m.items, &n.name) {
                self.line = n.line;
                let path = format!("{path}.{}", n.name);
                let child = self.instantiate(n, path, &[], Some((&[], sid)))?;
                self.scopes[sid].children.insert(n.name.clone(), child);
            }
        }
        if let Some(ps) = parent_scope {
            // after the instance's own children, as Icarus orders them
            self.port_width_warnings(m, sid, ps, &port_conns, &saved_file, inst_line)?;
        }
        self.file = saved_file;
        self.line = inst_line;
        Ok(sid)
    }

    /// Whether `items` (with generate constructs) instantiate module `name`.
    fn instantiates(items: &[Item], name: &str) -> bool {
        items.iter().any(|it| match it {
            Item::Instances { module, .. } => module == name,
            Item::GenFor { items, .. } | Item::GenBlock { items, .. } => Self::instantiates(items, name),
            Item::GenIf { then_items, else_items, .. } => {
                Self::instantiates(then_items, name) || Self::instantiates(else_items, name)
            }
            Item::GenCase { arms, .. } => arms.iter().any(|a| Self::instantiates(&a.2, name)),
            _ => false,
        })
    }

    /// The module `name` instantiated in scope `sid`: one declared in an enclosing module
    /// instance (innermost first), else a top-level one.
    pub(crate) fn find_module(&self, name: &str, sid: usize) -> Option<&'a Module> {
        let mut x = Some(sid);
        while let Some(s) = x {
            if let Some(m) = self.hier.scope_module.get(&s) {
                if let Some(n) = m.nested.iter().find(|n| n.name == name) {
                    return Some(n);
                }
            }
            x = self.scopes[s].parent;
        }
        self.hier.mods.get(name).copied()
    }

    /// The module instance scope that `sid` (a block or generate scope) belongs to.
    pub(crate) fn instance_scope(&self, mut sid: usize) -> usize {
        while !self.hier.scope_module.contains_key(&sid) {
            match self.scopes[sid].parent {
                Some(p) => sid = p,
                None => break,
            }
        }
        sid
    }

    /// Whether scope `sid` is in a program instance.
    pub(crate) fn in_program(&self, sid: usize) -> bool {
        self.hier.scope_module.get(&self.instance_scope(sid)).is_some_and(|m| m.program)
    }

    /// Whether an assignment target names a variable declared outside programs (programs
    /// should update those in the Reactive region, which is not modelled).
    pub(crate) fn lvalue_outside_program(&self, e: &Expr, sid: usize) -> bool {
        match e {
            Expr::Ident(n) => {
                let mut x = Some(sid);
                while let Some(s) = x {
                    if self.scopes[s].syms.contains_key(n) {
                        return !self.in_program(s);
                    }
                    x = self.scopes[s].parent;
                }
                false
            }
            Expr::Hier(p) => self.hier_scope(&p[..p.len() - 1], sid).is_some_and(|s| !self.in_program(s)),
            Expr::Index(b, _) | Expr::Range(b, ..) | Expr::IndexedRange(b, ..) | Expr::Member(b, _) => {
                self.lvalue_outside_program(b, sid)
            }
            Expr::Concat(v) => v.iter().any(|x| self.lvalue_outside_program(x, sid)),
            _ => false,
        }
    }

    pub(crate) fn push_job(&mut self, j: Job<'a>) {
        if let (Some(st), Job::Assign { .. }) = (self.nets.cur_strength, &j) {
            self.code.job_strength.insert(self.code.jobs.len(), st);
        }
        self.code.jobs.push(j);
    }

    /// Icarus's warnings for port connections whose width differs from the port's.
    pub(crate) fn port_width_warnings(
        &mut self,
        m: &'a Module,
        sid: usize,
        ps: usize,
        port_conns: &HashMap<String, Option<Expr>>,
        pfile: &str,
        line: u32,
    ) -> R<()> {
        let mut dirs: HashMap<&str, Dir> = HashMap::new();
        for it in &m.items {
            if let Item::Decl(d) = it {
                if let Some(dir) = d.dir {
                    for dl in &d.names {
                        dirs.insert(dl.name.as_str(), dir);
                    }
                }
            }
        }
        for (k, pn) in m.ports.iter().enumerate() {
            let Some(Some(e)) = port_conns.get(pn) else { continue };
            let Some(Sym::Sig(s)) = self.scopes[sid].syms.get(pn).cloned() else { continue };
            let info = self.sig(s);
            if self.sig_type(s).is_real() || info.depth != 0 {
                continue;
            }
            let pw = info.width;
            let et = self.type_of(e, ps)?;
            if et.is_real() {
                continue;
            }
            let ew = et.width();
            if pw == ew {
                continue;
            }
            let n = pw.abs_diff(ew);
            let detail = match (dirs.get(pn.as_str()), ew < pw) {
                (Some(Dir::Inout), true) => format!("Leaving {n} high bits of the port unconnected."),
                (Some(Dir::Inout), false) => format!("Leaving {n} high bits of the expression dangling."),
                (Some(Dir::Input), false) => format!("Pruning {n} high bits of the expression."),
                (_, true) => format!("Padding {n} high bits of the port."),
                (_, false) => format!("Padding {n} high bits of the expression."),
            };
            let w = format!(
                "{pfile}:{line}: warning: Port {} ({pn}) of module {} expects {pw} bit(s), given {ew}.\n{pfile}:{line}:        : {detail}",
                k + 1,
                m.name
            );
            if self.diag.warned.insert(w.clone()) {
                self.diag.warnings.push(w);
            }
        }
        Ok(())
    }

    /// Port widths of module `m` with these parameter overrides (evaluated in a scratch scope).
    pub(crate) fn port_widths(&mut self, m: &'a Module, overrides: &[(Option<String>, PVal)]) -> R<HashMap<String, u32>> {
        let saved = std::mem::replace(&mut self.file, m.file.clone());
        let sid = self.new_scope(format!("$ports.{}", m.name), None, m.file.clone());
        for it in &m.items {
            if let Item::Subroutine(f) = it {
                self.scopes[sid].funcs.insert(f.name.clone(), f);
            }
        }
        self.eval_params(m, overrides, sid)?;
        let mut w = HashMap::new();
        for it in &m.items {
            if let Item::Decl(d) = it {
                let pw = match d.kind {
                    NetKind::Integer => Some(32),
                    NetKind::Int(n) if d.range.is_none() => Some(n),
                    NetKind::Real | NetKind::Time => Some(64),
                    _ => match &d.range {
                        Some(r) => {
                            let (a, b) = self.range_vals(&Some(r.clone()), sid)?;
                            Some(((a - b).abs() + 1) as u32)
                        }
                        None => None,
                    },
                };
                for dl in &d.names {
                    if m.ports.contains(&dl.name) {
                        match pw {
                            Some(p) => {
                                w.insert(dl.name.clone(), p);
                            }
                            None => {
                                w.entry(dl.name.clone()).or_insert(1);
                            }
                        }
                    }
                }
            }
        }
        self.file = saved;
        Ok(w)
    }

    /// The part of terminal `e` that instance `k` (from the left) of an `n`-instance array gets:
    /// slices of `pw` bits, the leftmost instance taking the most significant one.
    pub(crate) fn array_slice(&mut self, e: &Expr, k: u32, n: u32, pw: u32, sid: usize) -> R<Expr> {
        self.slice_bits(e, ((n - 1 - k) * pw) as i64, pw as i64, sid)
    }

    /// Bits `[off, off+w)` (counted from the LSB) of `e`, as an expression that is still an
    /// lvalue when `e` is one.
    pub(crate) fn slice_bits(&mut self, e: &Expr, off: i64, w: i64, sid: usize) -> R<Expr> {
        let num = |v: i64| -> Expr {
            let e = Expr::Num(ferrooxide_front::ast::dec(v.unsigned_abs()));
            if v < 0 {
                Expr::Unary(UnOp::Neg, Box::new(e))
            } else {
                e
            }
        };
        // declared index range of a plain signal
        let decl = |me: &Self, b: &Expr| -> Option<(i64, i64)> {
            let sym = match b {
                Expr::Ident(nm) => me.lookup(nm, sid),
                Expr::Hier(p) => me.lookup_hier(p, sid),
                _ => None,
            };
            match sym {
                Some(Sym::Sig(s)) if me.sig(s).depth == 0 => Some((me.sig(s).msb, me.sig(s).lsb)),
                _ => None,
            }
        };
        let off_of = |(msb, lsb): (i64, i64), i: i64| if msb >= lsb { i - lsb } else { lsb - i };
        match e {
            Expr::Range(b, m, l) if decl(self, b).is_some() => {
                if let (Some(m), Some(l)) = (self.const_value(m, sid), self.const_value(l, sid)) {
                    let d = decl(self, b).unwrap();
                    let lo = off_of(d, m).min(off_of(d, l));
                    return self.slice_bits(&b.clone(), lo + off, w, sid);
                }
            }
            Expr::Index(b, i) if decl(self, b).is_some() && off == 0 && w == 1 => return Ok(e.clone()),
            Expr::Concat(parts) => {
                // walk from the LSB part; collect the pieces that overlap the slice
                let mut pieces = Vec::new();
                let mut base = 0i64;
                for p in parts.iter().rev() {
                    let pw = self.type_of(p, sid)?.width() as i64;
                    let (s0, s1) = (off.max(base), (off + w).min(base + pw));
                    if s0 < s1 {
                        let piece =
                            if s0 == base && s1 == base + pw { p.clone() } else { self.slice_bits(p, s0 - base, s1 - s0, sid)? };
                        pieces.push(piece);
                    }
                    base += pw;
                }
                pieces.reverse();
                return Ok(if pieces.len() == 1 { pieces.pop().unwrap() } else { Expr::Concat(pieces) });
            }
            _ => {}
        }
        let d = decl(self, e).unwrap_or((self.type_of(e, sid)?.width() as i64 - 1, 0));
        let idx = |o: i64| if d.0 >= d.1 { d.1 + o } else { d.1 - o };
        Ok(if w == 1 {
            Expr::Index(Box::new(e.clone()), Box::new(num(idx(off))))
        } else {
            Expr::Range(Box::new(e.clone()), Box::new(num(idx(off + w - 1))), Box::new(num(idx(off))))
        })
    }
}
