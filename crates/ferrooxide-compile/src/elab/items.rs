//! Module items: the walk over a module or generate scope.

use crate::*;

impl<'a> Compiler<'a> {
    /// Port `name` was declared without a range, its net or variable declaration (at `line`)
    /// with [a:b]: an error, or with `-gno-io-range-error` a warning (as the reference).
    pub(super) fn scalar_port(&mut self, name: &str, line: u32, a: i64, b: i64) -> R<()> {
        let msg = format!("Scalar port ``{name}'' has a vectored net declaration [{a}:{b}].");
        if !self.dialect.io_range_warn {
            self.line = line;
            return self.err(msg);
        }
        let w = format!("{}: warning: {msg}", ferrooxide_front::loc_str(&self.file, line));
        if !self.dialect.no_anachronisms && self.diag.warned.insert(w.clone()) {
            self.diag.warnings.push(w);
        }
        Ok(())
    }

    /// Rejects names declared twice among `decls` (the declarations of one scope, or of one
    /// subroutine; IEEE 1364-2005 12.3.3, 1800-2017 23.2.2.1): at most one direction and one
    /// net or variable declaration each; a direction declaration with a data type declares
    /// the port completely; with both, the ranges are the same (a typed declaration, or one
    /// without a range, goes only with a direction declaration without a range).
    pub(crate) fn check_redeclared<'d>(&mut self, decls: impl Iterator<Item = &'d Decl>, sid: usize) -> R<()> {
        let mut seen: HashMap<&str, (Option<&Decl>, Option<&Decl>)> = HashMap::new();
        for d in decls {
            if d.kind() == NetKind::Genvar {
                continue;
            }
            for dl in &d.names {
                let e = seen.entry(dl.name.as_str()).or_default();
                let slot = if d.dir.is_some() { &mut e.0 } else { &mut e.1 };
                if slot.is_some() {
                    self.line = dl.line;
                    let what = if d.dir.is_some() { "port " } else { "" };
                    return self.err(format!("{what}'{}' is declared more than once", dl.name));
                }
                *slot = Some(d);
                let (Some(p), Some(v)) = *e else { continue };
                self.line = dl.line;
                let lines = format!("lines {} and {}", p.line.min(v.line), p.line.max(v.line));
                if p.explicit_kind {
                    return self.err(format!(
                        "port '{}' is declared twice ({lines}): a port declaration with a data type declares it completely",
                        dl.name
                    ));
                }
                if p.type_param().is_some() || v.type_param().is_some() || !p.range_written {
                    continue; // (a range on the net only: the scalar port check)
                }
                // (ranges that cannot be evaluated here, e.g. with a subroutine's own
                // parameters, are not compared)
                let same = v.range_written && {
                    let mut ranges = |d: &Decl| -> Option<Vec<(i64, i64)>> {
                        let mut r = Vec::new();
                        for (a, b) in d.range().iter().chain(d.packed()) {
                            r.push((self.const_int(a, sid).ok()?, self.const_int(b, sid).ok()?));
                        }
                        Some(r)
                    };
                    match (ranges(p), ranges(v)) {
                        (Some(a), Some(b)) => a == b,
                        _ => true,
                    }
                };
                if !same {
                    return self.err(format!("port '{}': its declarations ({lines}) have different ranges", dl.name));
                }
            }
        }
        Ok(())
    }

    /// The items of a module (`module_level`; `port_conns`: its ports' connections in the
    /// parent scope) or generate scope `sid`, in phases: 1. subroutines, parameters, classes;
    /// 2. declarations; 3. implicit nets, path delays, static subroutines; 4. the rest.
    pub(crate) fn elab_items(
        &mut self,
        items: &'a [Item],
        sid: usize,
        module_level: bool,
        port_conns: &HashMap<String, Option<Expr>>,
        parent_scope: Option<usize>,
    ) -> R<()> {
        self.scope_defs(items, sid, module_level)?;
        let port_dirs = self.declarations(items, sid, module_level, port_conns, parent_scope)?;
        // function and task ports declared without a range that get one later: warned about
        // (as the reference does) by subroutine name
        let mut subs: Vec<&SubroutineDecl> = items.iter().filter_map(|it| if let Item::Subroutine(f) = it { Some(f) } else { None }).collect();
        subs.sort_by(|a, b| a.name.cmp(&b.name));
        for f in subs {
            let written = if f.decls_written.is_empty() { &f.decls } else { &f.decls_written };
            self.check_redeclared(written.iter(), sid)?;
            for (n, l) in &f.scalar_vec_ports {
                let Some((a, b)) = f.decls.iter().find(|d| d.dir.is_some() && d.names.iter().any(|x| &x.name == n)).and_then(|d| d.range()) else { continue };
                let (a, b) = (self.const_int(&a, sid)?, self.const_int(&b, sid)?);
                self.scalar_port(n, *l, a, b)?;
            }
        }
        // 3b. implicit nets: undeclared names on the left of continuous assignments and as gate
        // terminals become 1-bit wires
        let mut implicit: Vec<&String> = Vec::new();
        fn lhs_names<'e>(e: &'e Expr, out: &mut Vec<&'e String>) {
            match &e.kind {
                ExprKind::Ident(n) => out.push(n),
                ExprKind::Concat(v) => v.iter().for_each(|x| lhs_names(x, out)),
                _ => {}
            }
        }
        // (IEEE 1800-2017 6.10: also the names used in the terminals of primitive instances and
        // in the expressions connected to ports; a name connected alone to a module's port
        // gets the port's width, `declare_port`)
        fn names_in<'e>(e: &'e Expr, out: &mut Vec<&'e String>) {
            match &e.kind {
                ExprKind::Ident(n) => out.push(n),
                ExprKind::Concat(v) => v.iter().for_each(|x| names_in(x, out)),
                ExprKind::Unary(_, a) => names_in(a, out),
                ExprKind::Binary(_, a, b) => {
                    names_in(a, out);
                    names_in(b, out);
                }
                ExprKind::Cond(a, b, c) => {
                    names_in(a, out);
                    names_in(b, out);
                    names_in(c, out);
                }
                ExprKind::Index(a, i) => {
                    names_in(a, out);
                    names_in(i, out);
                }
                _ => {}
            }
        }
        for it in items {
            match it {
                Item::ContAssign { lhs, .. } => lhs_names(lhs, &mut implicit),
                Item::Gate { terms, .. } => terms.iter().for_each(|t| lhs_names(t, &mut implicit)),
                Item::Instances { module, insts, .. } => {
                    let udp = self.find_module(module, sid).is_some_and(|m| m.udp.is_some());
                    for inst in insts {
                        for c in &inst.conns {
                            if let Conn::Positional(Some(e)) | Conn::Named(_, Some(e)) = c {
                                if udp || !matches!(e.kind, ExprKind::Ident(_)) {
                                    names_in(e, &mut implicit);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for n in implicit {
            if self.lookup(n, sid).is_none() {
                self.implicit_net(n, 1, sid)?;
            }
        }
        // 3b'. specify path delays on output ports
        if module_level {
            self.check_paths(items, sid)?;
        }
        if module_level && self.dialect.specify {
            self.path_delays(items, sid, &port_dirs)?;
        }
        // 3c. static tasks and functions, so hierarchical names can reach their variables
        for it in items {
            if let Item::Subroutine(f) = it {
                if !f.automatic {
                    self.func_instance(f, sid, &f.name)?;
                }
            }
        }
        // 4. everything else; interface instances first, so that modules can be connected to
        // interfaces declared after them
        let mut genblk = 0;
        let is_iface = |it: &Item| matches!(it, Item::Instances { module, .. } if self.find_module(module, sid).is_some_and(|m| m.interface));
        let order: Vec<&'a Item> = items.iter().filter(|it| is_iface(it)).chain(items.iter().filter(|it| !is_iface(it))).collect();
        for it in order {
            self.elab_item(it, sid, &mut genblk)?;
        }
        Ok(())
    }

    /// 1. subroutines, classes and local parameters of generate scopes
    fn scope_defs(&mut self, items: &'a [Item], sid: usize, module_level: bool) -> R<()> {
        for it in items {
            if let Item::Subroutine(f) = it {
                self.scopes[sid].funcs.insert(f.name.clone(), f);
            }
        }
        if !module_level {
            for it in items {
                if let Item::TypeParam { name, default: Some(d), line, .. } = it {
                    self.line = *line;
                    let t = Rc::new(self.resolve_type(d, sid)?);
                    self.scopes[sid].syms.insert(name.clone(), Sym::Type(t));
                }
                if let Item::Param(p) = it {
                    self.line = p.line;
                    let v = self.param_value(p, None, sid)?;
                    self.insert_param(p, v, sid)?;
                if p.range().is_some() {
                    let r = self.range_vals(&p.range(), sid)?;
                    self.scopes[sid].param_ranges.insert(p.name.clone(), r);
                }
                }
            }
        }
        self.register_defparams(items, sid)?;
        // (after the parameters, which property types may use; the ids first, so that a
        // property can be a handle of a class declared after it)
        let mut ids = Vec::new();
        for it in items {
            if let Item::Class(c) = it {
                let id = self.reserve_class(c, &c.name, sid);
                self.classes.by_name.insert((sid, c.name.clone()), id);
                ids.push(id);
            }
        }
        let mut k = 0;
        for it in items {
            if let Item::Class(c) = it {
                // (a class with a parameter without a default exists only as specializations)
                if !Self::needs_params(c) {
                    self.declare_class(c, sid, &c.name, &[], ids[k])?;
                }
                k += 1;
            }
        }
        // the specializations of parameterized classes this scope uses
        for it in items {
            if let Item::ClassSpec { name, class, args, line } = it {
                self.line = *line;
                self.class_spec(name, class, args, sid)?;
            }
        }
        Ok(())
    }

    /// 4. an item other than a declaration (`genblk`: the number of the last generate block)
    fn elab_item(&mut self, it: &'a Item, sid: usize, genblk: &mut usize) -> R<()> {
        match it {
            Item::Decl(_)
            | Item::Param(_)
            | Item::TypeParam { .. }
            | Item::Subroutine(_)
            | Item::Class(_)
            | Item::ClassSpec { .. }
            | Item::Defparam { .. }
            | Item::SpecPath { .. } => {}
            Item::ContAssign { delay, lhs, rhs, strength, line } => {
                self.nets.cur_strength = *strength;
                // (a whole unpacked array becomes an alias or one assignment per element in
                // `alias_assigns`)
                self.push_job(Job::Assign { lhs: lhs.clone(), rhs: rhs.clone(), delay: delay.clone(), scope: sid, line: *line });
                self.nets.cur_strength = None;
            }
            Item::Process { kind, body, line } => {
                self.code.jobs.push(Job::Proc { kind: *kind, body, scope: sid, line: *line });
            }
            Item::Instances { module, params, insts, line } => self.instances(module, params, insts, *line, sid)?,
            Item::Assert(a) => self.code.jobs.push(Job::Assert { a, scope: sid }),
            Item::ElabTask { name, args, line } => self.elab_task(name, args, *line, sid)?,
            Item::GenFor { .. } => self.gen_for(it, sid, genblk)?,
            Item::GenIf { .. } => self.gen_if(it, sid, genblk)?,
            Item::GenCase { .. } => self.gen_case(it, sid, genblk)?,
            Item::GenBlock { name, items: its, line } => {
                self.line = *line;
                *genblk += 1;
                self.gen_scope(its, name, *genblk, sid)?;
            }
            Item::Gate { kind, terms, line, dims, strength, delay, .. } => {
                self.line = *line;
                self.nets.cur_strength = *strength;
                self.nets.cur_delay = delay.clone();
                let r = self.gate_items(kind, terms, dims, sid, *line);
                self.nets.cur_strength = None;
                self.nets.cur_delay = None;
                r?;
            }
        }
        Ok(())
    }

    /// Instances of module or primitive `module` (with parameter values `params`).
    fn instances(&mut self, module: &str, params: &'a [Conn], insts: &'a [Instance], line: u32, sid: usize) -> R<()> {
        self.line = line;
        let m = match self.find_module(module, sid) {
            Some(m) => m,
            None => return self.err(format!("unknown module '{module}'")),
        };
        if m.udp.is_some() {
            // a user-defined primitive: positional terminals, an optional delay
            // (rise and fall at most: its output is never z)
            if params.len() > 2 {
                return self.err(format!("primitive '{module}' takes at most two delays, not {}", params.len()));
            }
            let delay = match params.first() {
                Some(Conn::Positional(Some(e))) => Some(e.clone()),
                _ => None,
            };
            for inst in insts {
                let mut terms = Vec::new();
                for c in &inst.conns {
                    match c {
                        Conn::Positional(Some(e)) | Conn::Named(_, Some(e)) => terms.push(e.clone()),
                        _ => return self.err("unconnected UDP terminal"),
                    }
                }
                if terms.len() != m.ports.len() {
                    return self.err(format!("primitive '{module}' needs {} terminals", m.ports.len()));
                }
                self.code.jobs.push(Job::Udp { module: m, terms, delay: delay.clone(), scope: sid, line: inst.line });
            }
            return Ok(());
        }
        let mut ovs = Vec::new();
        for p in params {
            match p {
                Conn::Positional(Some(e)) => ovs.push((None, self.pval(e, sid)?)),
                Conn::Positional(None) => ovs.push((None, PVal::Default)),
                Conn::Named(n, Some(e)) => ovs.push((Some(n.clone()), self.pval(e, sid)?)),
                _ => {}
            }
        }
        for inst in insts {
            self.line = inst.line;
            if inst.name.starts_with('$') {
                return self.err(format!("Instantiation of module {} requires an instance name.", m.name));
            }
            if inst.dims.len() > 1 {
                return self.err("multi-dimensional arrays of instances are not supported");
            }
            if let Some((l, r)) = inst.dims.first() {
                // an array of instances: name[i] for i from l to r, terminals sliced
                let (l, r) = (self.const_int(l, sid)?, self.const_int(r, sid)?);
                let n = ((l - r).abs() + 1) as u32;
                let pws = self.port_widths(m, &ovs)?;
                self.hier.inst_arrays.insert((sid, inst.name.clone()), (l, r));
                // elaborated lowest index first, like Icarus
                for i in l.min(r)..=l.max(r) {
                    let k = (i - l).unsigned_abs() as u32;
                    let mut conns = Vec::new();
                    for (pos, c) in inst.conns.iter().enumerate() {
                        let (port, e) = match c {
                            Conn::Positional(e) => (m.ports.get(pos).cloned().unwrap_or_default(), e),
                            Conn::Named(nm, e) => (nm.clone(), e),
                            Conn::Wildcard => {
                                conns.push(Conn::Wildcard);
                                continue;
                            }
                        };
                        let e = match e {
                            Some(e) => {
                                let pw = pws.get(&port).copied().unwrap_or(1);
                                let tw = self.type_of(e, sid)?.width();
                                if n > 1 && tw == pw * n {
                                    Some(self.array_slice(e, k, n, pw, sid)?)
                                } else if tw == pw || tw < pw * n {
                                    Some(e.clone())
                                } else {
                                    return self.err(format!(
                                        "port '{port}' of instance array '{}': {tw} bits do not fit {n} x {pw}",
                                        inst.name
                                    ));
                                }
                            }
                            None => None,
                        };
                        conns.push(match c {
                            Conn::Positional(_) => Conn::Positional(e),
                            _ => Conn::Named(port, e),
                        });
                    }
                    // the generated connections live as long as the elaboration
                    let conns: &'a [Conn] = Box::leak(conns.into_boxed_slice());
                    let iname = format!("{}[{}]", inst.name, i);
                    let path = format!("{}.{}", self.scopes[sid].name, iname);
                    let child = self.instantiate(m, path, &ovs, Some((conns, sid)))?;
                    self.scopes[sid].children.insert(iname, child);
                }
                continue;
            }
            let path = format!("{}.{}", self.scopes[sid].name, inst.name);
            let child = self.instantiate(m, path, &ovs, Some((&inst.conns, sid)))?;
            self.scopes[sid].children.insert(inst.name.clone(), child);
        }
        Ok(())
    }

    /// An elaboration system task (IEEE 1800-2017 20.11) reached in scope `sid`: its message,
    /// as the reference words it; `$error` and `$fatal` make elaboration fail.
    fn elab_task(&mut self, name: &str, args: &[Option<Expr>], line: u32, sid: usize) -> R<()> {
        if self.diag.elab_fatal {
            return Ok(());
        }
        self.line = line;
        // (`$fatal`'s first argument, when not the message, is the finish number)
        let args = match (name, args.first()) {
            ("$fatal", Some(Some(e))) if !matches!(e.kind, ExprKind::Str(_)) => &args[1..],
            _ => args,
        };
        let scope = self.scopes[sid].name.rsplit('.').next().unwrap_or_default().to_string();
        let msg = if args.is_empty() {
            String::new()
        } else {
            self.code.pb.push(PState { proc: NONE, ..Default::default() });
            let fa = self.fmt_args(args, sid);
            let st = self.code.pb.pop().unwrap();
            let fa = fa?;
            if let Err(m) = rt::vm::eval_const(&st.code, &mut self.prog.mem, &self.prog.signals) {
                return self.err(m);
            }
            let texp = self.texp(sid);
            String::from_utf8_lossy(&rt::fmt::format_args(&fa, &self.prog.mem, &scope, 'd', texp)).into_owned()
        };
        let kind = match name {
            "$fatal" => "FATAL",
            "$error" => "ERROR",
            "$warning" => "WARNING",
            _ => "INFO",
        };
        let pad = " ".repeat(kind.len() + 2);
        let sep = if msg.is_empty() { "" } else { " " };
        self.diag.elab_msgs.push(format!("{kind}: {}:{sep}{msg}\n{pad}During elaboration  Scope: {scope}", self.loc()));
        if matches!(name, "$fatal" | "$error") {
            self.diag.elab_errors.push((self.file.clone(), line));
            self.diag.elab_fatal |= name == "$fatal";
        }
        Ok(())
    }

    /// A named block `name` (its child scope `key`) may be declared in scope `sid`: no
    /// signal, parameter or other block there has the name (IEEE 1800-2017 23.9).
    pub(crate) fn scope_name_free(&mut self, name: &str, key: &str, sid: usize) -> R<()> {
        if self.scopes[sid].syms.contains_key(name) || self.scopes[sid].children.contains_key(key) {
            return self.err(format!("'{name}' is already declared in this scope"));
        }
        Ok(())
    }
}
