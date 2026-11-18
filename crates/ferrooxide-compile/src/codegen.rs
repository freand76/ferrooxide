//! Code generation: processes, instructions, the jobs collected during elaboration.

use crate::*;

impl<'a> Compiler<'a> {
    pub(crate) fn emit(&mut self, mut i: Instr) -> usize {
        // a store into a 2-state variable: x/z bits of the value become 0 first
        if let Instr::Store { t, s, mode } = i {
            use rt::Target;
            let sig = match t {
                Target::Whole { sig }
                | Target::Part { sig, .. }
                | Target::DynPart { sig, .. }
                | Target::Elem { sig, .. }
                | Target::ElemK { sig, .. }
                | Target::ElemPart { sig, .. }
                | Target::ElemDynPart { sig, .. } => sig,
            };
            if !self.nets.alias_vars.is_empty() {
                let mut v = sig;
                loop {
                    if self.nets.alias_vars.contains(&v) {
                        if self.sigs.mem_error.is_none() {
                            let name = self.sig(v).name.rsplit('.').next().unwrap_or("").to_string();
                            self.sigs.mem_error = self
                                .err::<()>(format!(
                                    "Cannot perform procedural assignment to variable '{name}' because it is also continuously assigned."
                                ))
                                .err();
                        }
                        break;
                    }
                    match self.nets.view_of.get(&v) {
                        Some(&b) => v = b,
                        None => break,
                    }
                }
            }
            // (a value converted from a real keeps its x from inf/nan, as in the reference
            // simulator: ICARUS_NOTES.md)
            let from_real = {
                let code = &self.code.pb.last().unwrap().code;
                let mut v = s;
                let mut found = false;
                for ins in code.iter().rev().take(4) {
                    match *ins {
                        Instr::Resize { d, s: x, .. } | Instr::Copy { d, s: x, .. } if d == v => v = x,
                        Instr::RToI { d, .. } if d == v => {
                            found = true;
                            break;
                        }
                        _ => break,
                    }
                }
                found
            };
            if self.sigs.two_state.contains(&self.canon(sig)) && !from_real {
                let w = match t {
                    Target::Part { w, .. } | Target::DynPart { w, .. } | Target::ElemPart { w, .. } | Target::ElemDynPart { w, .. } => w,
                    _ => self.sig(sig).width,
                };
                let d = self.tmp(w);
                self.code.pb.last_mut().unwrap().code.push(Instr::Un { op: rt::UnOp::To2, d, s, w });
                i = Instr::Store { t, s: d, mode };
            }
        }
        let c = &mut self.code.pb.last_mut().unwrap().code;
        c.push(i);
        c.len() - 1
    }

    pub(crate) fn here(&self) -> u32 {
        self.code.pb.last().unwrap().code.len() as u32
    }

    pub(crate) fn patch(&mut self, at: usize, to: u32) {
        match &mut self.code.pb.last_mut().unwrap().code[at] {
            Instr::Jmp { to: t } | Instr::Jz { to: t, .. } => *t = to,
            _ => panic!("patching a non-jump"),
        }
    }

    pub(crate) fn begin_proc(&mut self, name: String, class: ProcClass) -> u32 {
        let id = self.prog.procs.len() as u32;
        self.code.proc_scope.push(self.code.job_scope);
        self.prog.procs.push(ProcDef { name, code: Vec::new(), class, parent: NONE, fusible: false, kick0: false });
        self.code.pb.push(PState { proc: id, ..Default::default() });
        self.code.scratch_saved.push(self.code.scratch_top);
        self.code.scratch_top = 0;
        id
    }

    pub(crate) fn end_proc(&mut self) {
        if let Err(e) = self.finish_subs() {
            if self.sigs.mem_error.is_none() {
                self.sigs.mem_error = Some(e);
            }
        }
        // nested processes ($monitor/$strobe bodies) restart at 0: the enclosing process's
        // temporaries are dead whenever the nested one runs (it runs in the postponed region)
        self.code.scratch_top = self.code.scratch_saved.pop().unwrap().max(self.code.scratch_top);
        let st = self.code.pb.pop().unwrap();
        self.prog.procs[st.proc as usize].code = st.code;
    }

    /// An event control: (signal, edge, array element or NONE) triggers.
    pub(crate) fn make_ctl(&mut self, mut triggers: Vec<(u32, rt::Edge, u32)>) -> u32 {
        triggers.sort_by_key(|t| (t.0, t.2, t.1 as u8));
        triggers.dedup();
        let proc = self.code.pb.last().unwrap().proc;
        let id = self.prog.ctls.len() as u32;
        self.prog.ctls.push(rt::EventCtl { proc, triggers });
        id
    }

    /// Register a named block / task body; returns its registry index.
    pub(crate) fn open_block(&mut self, path: String) -> Option<usize> {
        let proc = self.code.pb.last().map(|p| p.proc).unwrap_or(NONE);
        if proc == NONE {
            return None; // constant evaluation
        }
        let start = self.here();
        self.code.block_reg.push((path, rt::BlockDef { proc, start, end: start, children: Vec::new() }));
        let i = self.code.block_reg.len() - 1;
        self.code.open_blocks.push(i);
        Some(i)
    }

    pub(crate) fn close_block(&mut self, i: Option<usize>) {
        if let Some(i) = i {
            self.code.block_reg[i].1.end = self.here();
            let top = self.code.open_blocks.pop();
            debug_assert_eq!(top, Some(i));
        }
    }

    /// Map the `disable` targets to the registered blocks (first matching scope outwards).
    pub(crate) fn resolve_disables(&mut self) {
        let pending = std::mem::take(&mut self.code.pending_disables);
        for (target, cands, file, line) in pending {
            let mut ids = Vec::new();
            for c in &cands {
                ids = (0..self.code.block_reg.len() as u32).filter(|&i| &self.code.block_reg[i as usize].0 == c).collect();
                if !ids.is_empty() {
                    break;
                }
            }
            if ids.is_empty() && !cands.iter().any(|c| c.starts_with("$force:")) {
                let n = cands.first().map(|c| c.rsplit('.').next().unwrap_or(c).to_string()).unwrap_or_default();
                let m = format!("{file}:{line}: warning: disable: no block or task named '{n}'");
                if self.diag.warned.insert(m.clone()) {
                    self.diag.warnings.push(m);
                }
            }
            self.prog.disables[target as usize] = ids;
        }
        self.prog.blocks = self.code.block_reg.iter().map(|b| b.1.clone()).collect();
    }

    /// Assignment targets go to the job's driver signal when the net has several drivers.
    /// A view of a redirected net becomes a view of the job's driver signal.
    pub(crate) fn redir(&mut self, s: u32) -> u32 {
        let c = self.canon(s);
        let Some(&d) = self.nets.redirect.get(&c) else { return s };
        if c == s {
            return d;
        }
        if let Some(&v) = self.nets.drv_views.get(&(s, d)) {
            return v;
        }
        let i = self.sig(s).clone();
        let v = self.new_view(d, i.name, i.msb, i.lsb, i.signed);
        self.prog.names.pop();
        self.nets.drv_views.insert((s, d), v);
        v
    }

    /// Signals an lvalue writes (whole signals; part selects count as the signal).
    pub(crate) fn lvalue_bases(&self, e: &Expr, sid: usize, out: &mut Vec<u32>) {
        match e {
            Expr::Ident(n) => {
                if let Some(Sym::Sig(s)) = self.lookup(n, sid) {
                    out.push(self.canon(s));
                }
            }
            Expr::Hier(p) => {
                if let Some(Sym::Sig(s)) = self.lookup_hier(p, sid) {
                    out.push(self.canon(s));
                }
            }
            Expr::Index(a, _) | Expr::Range(a, _, _) | Expr::IndexedRange(a, _, _, _) => self.lvalue_bases(a, sid, out),
            Expr::Concat(v) => {
                for x in v {
                    self.lvalue_bases(x, sid, out);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn job_targets(&self, job: &Job) -> Vec<u32> {
        let mut v = Vec::new();
        match job {
            Job::Assign { lhs, scope, .. } | Job::PortOut { lhs, scope, .. } => self.lvalue_bases(lhs, *scope, &mut v),
            Job::PortIn { child_sig, .. } => v.push(*child_sig),
            Job::Udp { terms, scope, .. } => self.lvalue_bases(&terms[0], *scope, &mut v),
            Job::Proc { .. } | Job::Init { .. } => {}
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    /// A final procedure runs in zero time: no nonblocking assignments, delays, event or
    /// wait controls, task calls (IEEE 1800-2017 9.2.3).
    fn check_final(&mut self, s: &Stmt, sid: usize) -> R<()> {
        let err = |c: &mut Self, line: u32, what: &str| -> R<()> {
            c.line = line;
            c.err(format!("final procedures cannot {what}."))
        };
        match s {
            Stmt::Assign { nonblocking: true, line, .. } => err(self, *line, "have non blocking assignment statements"),
            Stmt::Assign { timing: Some(_), line, .. } => err(self, *line, "have delay statements"),
            Stmt::Timed { timing: Timing::Delay(_), line, .. } => err(self, *line, "have delay statements"),
            Stmt::Timed { line, .. } => err(self, *line, "have event statements"),
            Stmt::Wait { line, .. } => err(self, *line, "have wait statements"),
            Stmt::TaskCall { name, line, .. } if self.lookup_func(name, sid).is_some_and(|(f, _)| f.is_task) => {
                err(self, *line, "enable/call tasks")
            }
            Stmt::Block { stmts, .. } => stmts.iter().try_for_each(|x| self.check_final(x, sid)),
            Stmt::If { then_s, else_s, .. } => {
                self.check_final(then_s, sid)?;
                else_s.iter().try_for_each(|x| self.check_final(x, sid))
            }
            Stmt::Case { arms, default, .. } => {
                arms.iter().try_for_each(|a| self.check_final(&a.1, sid))?;
                default.iter().try_for_each(|x| self.check_final(x, sid))
            }
            Stmt::For { body, .. }
            | Stmt::While { body, .. }
            | Stmt::DoWhile { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::Forever { body, .. } => self.check_final(body, sid),
            _ => Ok(()),
        }
    }

    pub(crate) fn codegen_jobs(&mut self) -> R<()> {
        let jobs = std::mem::take(&mut self.code.jobs);
        let jobs = self.alias_assigns(jobs);
        // nets with more than one continuous driver are resolved from per-driver signals
        let mut count: HashMap<u32, u32> = HashMap::new();
        for job in &jobs {
            for s in self.job_targets(job) {
                *count.entry(s).or_default() += 1;
            }
        }
        let multi: HashSet<u32> = count
            .into_iter()
            .filter(|&(s, n)| {
                let i = self.sig(s);
                let special = self.nets.net_types.contains_key(&s) || self.nets.pulls.contains_key(&s);
                (n > 1 || special) && i.kind == SigKind::Wire && i.depth == 0 && i.alias_of.is_none()
            })
            .map(|(s, _)| s)
            .collect();
        // linked nets (tran, inout ports on parts of nets) and nets with drivers of a
        // non-default strength are always resolved
        let mut multi = multi;
        let job_strength = std::mem::take(&mut self.code.job_strength);
        for (k, job) in jobs.iter().enumerate() {
            if job_strength.get(&k).is_some_and(|st| *st != (6, 6)) {
                for s in self.job_targets(job) {
                    let i = self.sig(s);
                    if i.kind == SigKind::Wire && i.depth == 0 && i.alias_of.is_none() {
                        multi.insert(s);
                    }
                }
            }
        }
        for l in &self.nets.links {
            for s in [l.a, l.b] {
                if self.sig(s).alias_of.is_none() {
                    multi.insert(s);
                }
            }
        }
        // special nets nothing drives: their resolved value is constant
        let undriven: Vec<u32> = self
            .nets.net_types
            .keys()
            .chain(self.nets.pulls.keys())
            .copied()
            .filter(|s| !multi.contains(s))
            .collect();
        for s in undriven {
            if let Some(c) = self.pull_const(s) {
                let i = self.sig(s).clone();
                let n = rt::nw(i.width);
                for k in 0..n {
                    self.prog.mem[i.slot as usize + k] = c.a[k];
                    self.prog.mem[i.slot as usize + n + k] = c.b[k];
                }
            }
        }
        let mut drv_strength: HashMap<u32, (u8, u8)> = HashMap::new();
        for (jk, job) in jobs.iter().enumerate() {
            self.nets.redirect.clear();
            for s in self.job_targets(job) {
                if multi.contains(&s) {
                    let i = self.sig(s).clone();
                    let k = self.nets.drivers.get(&s).map(|v| v.len()).unwrap_or(0);
                    let d = self.new_signal(format!("{}$drv{k}", i.name), i.width, i.signed, 0, i.msb, i.lsb, 0, SigKind::Wire);
                    self.prog.names.pop(); // driver signals are not dumped
                    if let Some(&st) = job_strength.get(&jk) {
                        drv_strength.insert(d, st);
                    }
                    self.nets.drivers.entry(s).or_default().push(d);
                    self.nets.redirect.insert(s, d);
                }
            }
            self.codegen_job(job)?;
        }
        self.nets.redirect.clear();
        let island = self.link_islands()?;
        // resolver processes: net = resolve(driver 0, driver 1, ...)
        let mut nets: Vec<(u32, Vec<u32>)> =
            std::mem::take(&mut self.nets.drivers).into_iter().filter(|(n, _)| !island.contains(n)).collect();
        nets.sort();
        for (net, drv) in nets {
            let i = self.sig(net).clone();
            let p = self.begin_proc(i.name.clone(), ProcClass::Normal);
            self.prog.procs[p as usize].fusible = true;
            let top = self.here();
            let nt = self.nets.net_types.get(&net).copied().unwrap_or(NetType::Wire);
            let rop = match nt {
                NetType::Wand => rt::BinOp::ResolveAnd,
                NetType::Wor => rt::BinOp::ResolveOr,
                _ => rt::BinOp::Resolve,
            };
            let mut v = self.sig(drv[0]).slot;
            let strengths = drv.iter().any(|d| drv_strength.get(d).is_some_and(|st| *st != (6, 6)));
            if strengths && rop == rt::BinOp::Resolve {
                // drivers of different strengths: the strongest wins (per bit)
                let list: Vec<(u32, u8, u8)> = drv
                    .iter()
                    .map(|d| {
                        let (s0, s1) = drv_strength.get(d).copied().unwrap_or((6, 6));
                        (self.sig(*d).slot, s0, s1)
                    })
                    .collect();
                let t = self.tmp(i.width);
                self.emit(Instr::ResolveStr { d: t, w: i.width, drv: list.into_boxed_slice() });
                v = t;
                self.prog.procs[p as usize].fusible = false;
            } else {
                for &d in &drv[1..] {
                    let t = self.tmp(i.width);
                    let b = self.sig(d).slot;
                    self.emit(Instr::Bin { op: rop, d: t, a: v, b, w: i.width });
                    v = t;
                }
            }
            // pulled nets: undriven (z) bits read as 0 / 1
            if let Some(c) = self.pull_const(net) {
                let c = self.const_slot(&c);
                let t = self.tmp(i.width);
                self.emit(Instr::Bin { op: rt::BinOp::PullZ, d: t, a: v, b: c, w: i.width });
                v = t;
            }
            self.emit(Instr::Store { t: rt::Target::Whole { sig: net }, s: v, mode: rt::StoreMode::Blocking });
            let ctl = self.make_ctl(drv.iter().map(|&d| (d, rt::Edge::Any, NONE)).collect());
            self.emit(Instr::Wait { ctl });
            self.emit(Instr::Jmp { to: top });
            self.end_proc();
        }
        Ok(())
    }

    /// A continuous assignment of a constant to a whole net (or net driver) sets its initial
    /// value, so processes see it already at time 0, as with Icarus.
    pub(crate) fn const_assign_init(&mut self, lhs: &Expr, rhs: &Expr, sid: usize) -> R<()> {
        if !matches!(lhs, Expr::Ident(_) | Expr::Hier(_)) || !self.is_const(rhs, sid) || self.type_of(rhs, sid)?.is_real() {
            return Ok(());
        }
        let Some(expr::Base::Sig(s0)) = self.resolve_base(lhs, sid)? else { return Ok(()) };
        let s = self.redir(s0);
        let info = self.sig(s).clone();
        if info.depth != 0 || self.sig_type(s).is_real() || info.forced {
            return Ok(());
        }
        let c = self.const_eval(rhs, sid)?;
        let c = c.resize(info.width, c.signed);
        let n = rt::nw(info.width);
        for i in 0..n {
            self.prog.mem[info.slot as usize + i] = c.a[i];
            self.prog.mem[info.slot as usize + n + i] = c.b[i];
        }
        self.code.init_touch_net.push(s);
        if s != s0 {
            self.code.init_touch_net.push(s0);
        }
        Ok(())
    }

    /// The time-0 change events of initial values: a process that runs after all others have
    /// started.
    pub(crate) fn init_events(&mut self) {
        let mut v = std::mem::take(&mut self.code.init_touch_net);
        if self.dialect.v2005 {
            v.append(&mut self.code.init_touch);
        }
        let mut seen = HashSet::new();
        v.retain(|s| seen.insert(*s));
        if v.is_empty() {
            return;
        }
        self.code.job_scope = usize::MAX;
        self.begin_proc("$init".into(), ProcClass::Normal);
        for sig in v {
            self.emit(Instr::Touch { sig });
        }
        self.emit(Instr::Halt);
        self.end_proc();
    }

    pub(crate) fn codegen_job(&mut self, job: &Job<'a>) -> R<()> {
        // (only initial / always blocks start in scope order; continuous assignments and
        // port connections keep theirs)
        self.code.job_scope = match job {
            Job::Proc { scope, .. } => *scope,
            _ => usize::MAX - 1,
        };
        {
            match job {
                Job::Proc { kind, body, scope, line } => {
                    self.line = *line;
                    self.file = self.scopes[*scope].file.clone();
                    let class = if *kind == ProcKind::Final { ProcClass::Final } else { ProcClass::Normal };
                    let name = self.scopes[*scope].name.clone();
                    self.begin_proc(name, class);
                    match kind {
                        ProcKind::Initial | ProcKind::Final => {
                            if *kind == ProcKind::Final {
                                self.check_final(body, *scope)?;
                            }
                            self.stmt(body, *scope)?;
                            self.emit(Instr::Halt);
                        }
                        ProcKind::Always | ProcKind::AlwaysFf => {
                            if let Stmt::Timed { timing: Timing::Event(EventCtl::Star), .. } = body {
                                let p = self.code.pb.last().unwrap().proc;
                                self.prog.procs[p as usize].fusible = true;
                            }
                            let top = self.here();
                            self.stmt(body, *scope)?;
                            self.emit(Instr::Jmp { to: top });
                            if self.diag.lint_enabled {
                                if let Stmt::Timed { timing: Timing::Event(EventCtl::List(items)), body: inner, .. } = body {
                                    self.lint_sensitivity(items, inner, *scope, *line);
                                }
                            }
                        }
                        ProcKind::AlwaysComb | ProcKind::AlwaysLatch => {
                            let p = self.code.pb.last().unwrap().proc;
                            self.prog.procs[p as usize].fusible = *kind == ProcKind::AlwaysComb;
                            // IEEE 1800 9.2.2.2.1: what the block and the functions it calls read,
                            // except what the block writes itself
                            let mut reads = Vec::new();
                            self.subr.func_reads.set(true);
                            self.reads_stmt(body, *scope, &mut reads);
                            self.subr.func_reads.set(false);
                            let mut written = Vec::new();
                            self.blocking_writes(body, *scope, &mut written);
                            reads.retain(|(s, _)| !written.contains(s));
                            let ctl = self.make_ctl(reads.into_iter().map(|(s, e)| (s, rt::Edge::Any, e)).collect());
                            // 9.2.2.2.2: it runs once at time 0 after all initial and always
                            // procedures have started: it waits first and gets a time-0 wake
                            let top = self.here();
                            self.emit(Instr::Wait { ctl });
                            self.stmt(body, *scope)?;
                            self.emit(Instr::Jmp { to: top });
                            self.prog.procs[p as usize].kick0 = true;
                        }
                    }
                    self.end_proc();
                }
                Job::Init { lhs, rhs, scope, line } => {
                    self.line = *line;
                    self.file = self.scopes[*scope].file.clone();
                    let name = self.scopes[*scope].name.clone();
                    self.begin_proc(name, ProcClass::Normal);
                    let st = Stmt::Assign { lhs: lhs.clone(), rhs: rhs.clone(), nonblocking: false, timing: None, line: *line };
                    self.stmt(&st, *scope)?;
                    self.emit(Instr::Halt);
                    self.end_proc();
                }
                Job::Assign { lhs, rhs, delay, scope, line } => {
                    self.line = *line;
                    self.file = self.scopes[*scope].file.clone();
                    if delay.is_none() {
                        self.const_assign_init(lhs, rhs, *scope)?;
                    }
                    self.assign_proc(stmt::Lhs::Expr(lhs, *scope), stmt::Rhs::Expr(rhs, *scope), delay.as_ref())?;
                }
                Job::PortIn { child_sig, expr, scope, line } => {
                    self.line = *line;
                    self.file = self.scopes[*scope].file.clone();
                    self.assign_proc(stmt::Lhs::Sig(*child_sig), stmt::Rhs::Expr(expr, *scope), None)?;
                }
                Job::Udp { module, terms, delay, scope, line } => {
                    self.line = *line;
                    self.file = self.scopes[*scope].file.clone();
                    self.udp_instance(module, terms, delay.as_ref(), *scope)?;
                }
                Job::PortOut { lhs, child_sig, scope, line } => {
                    self.line = *line;
                    self.file = self.scopes[*scope].file.clone();
                    self.assign_proc(stmt::Lhs::Expr(lhs, *scope), stmt::Rhs::Sig(*child_sig), None)?;
                }
            }
        }
        Ok(())
    }
}
