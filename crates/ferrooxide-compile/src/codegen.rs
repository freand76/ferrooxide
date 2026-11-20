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

    /// Compile each module or package subroutine that is never called once, for its errors
    /// (a call inlines a subroutine, so an uncalled one would never be checked). The code
    /// and everything made for it are dropped again. Class methods are left out.
    pub(crate) fn check_uncalled_subs(&mut self) -> R<()> {
        let mut todo: Vec<(usize, &'a SubroutineDecl)> = Vec::new();
        for sid in 0..self.scopes.len() {
            if self.classes.of_scope.contains_key(&sid) {
                continue;
            }
            for (name, &f) in &self.scopes[sid].funcs {
                let called = self.subr.compiled.contains(&(sid, name.clone()));
                if !called {
                    todo.push((sid, f));
                }
            }
        }
        if todo.is_empty() {
            return Ok(());
        }
        todo.sort_by_key(|(s, f)| (self.scopes[*s].file.clone(), f.line, *s));
        let p = &self.prog;
        let lens = (p.mem.len(), p.signals.len(), p.procs.len(), p.ctls.len(), p.names.len(), p.strings.len(), p.locs.len(), p.classes.len());
        let mem_error = self.sigs.mem_error.take();
        let mut r = Ok(());
        for (sid, f) in todo {
            self.file = self.scopes[sid].file.clone();
            self.line = f.line;
            self.begin_proc(String::new(), ProcClass::Normal);
            let key = format!("{}$check", f.name);
            r = self.func_instance(f, sid, &key).and_then(|_| {
                let (fscope, ret) = {
                    let fi = &self.subr.func_inst[&(sid, key.clone())];
                    (fi.scope, fi.ret)
                };
                self.subr.inline_stack.push((sid, f.name.clone(), (sid, key.clone())));
                self.code.pb.last_mut().unwrap().rets.push((ret, Vec::new()));
                let r = self.stmt(&f.body, fscope);
                self.code.pb.last_mut().unwrap().rets.pop();
                self.subr.inline_stack.pop();
                r
            });
            self.end_proc();
            if r.is_err() {
                break;
            }
        }
        let p = &mut self.prog;
        p.mem.truncate(lens.0);
        p.signals.truncate(lens.1);
        p.procs.truncate(lens.2);
        p.ctls.truncate(lens.3);
        p.names.truncate(lens.4);
        p.strings.truncate(lens.5);
        p.locs.truncate(lens.6);
        p.classes.truncate(lens.7);
        self.sigs.mem_error = mem_error;
        r
    }

    pub(crate) fn begin_proc(&mut self, name: String, class: ProcClass) -> u32 {
        let id = self.prog.procs.len() as u32;
        self.code.proc_scope.push(self.code.job_scope);
        self.prog.procs.push(ProcDef { name, code: Vec::new(), class, parent: NONE, fusible: false, kick0: false, frame: Vec::new(), capture: false, fused: false });
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
        if !st.frame.is_empty() {
            self.prog.procs[st.proc as usize].frame = merge_ranges(st.frame);
        }
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
                let m = format!("{}: warning: disable: no block or task named '{n}'", ferrooxide_front::loc_str(&file, line));
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
    /// A whole one-dimensional unpacked array named by `e`: the signal and its indexes from
    /// left to right (the order elements pair up in an array assignment, IEEE 1800-2017
    /// 7.6). None for anything else.
    pub(crate) fn whole_array(&mut self, e: &Expr, sid: usize) -> R<Option<(u32, Vec<i64>)>> {
        if !matches!(e, Expr::Ident(_) | Expr::Hier(_)) {
            return Ok(None);
        }
        let Some(crate::expr::Base::Sig(s)) = self.resolve_base(e, sid)? else { return Ok(None) };
        let i = self.sig(s);
        if i.depth == 0 || self.sigs.md_dims.contains_key(&s) || i.real || i.string {
            return Ok(None);
        }
        let (lo, n) = (i.arr_lo, i.depth as i64);
        let idx = if self.sigs.desc_arrays.contains(&s) { (0..n).map(|k| lo + n - 1 - k).collect() } else { (0..n).map(|k| lo + k).collect() };
        Ok(Some((s, idx)))
    }

    /// The elements of the unpacked array named by `e` (a whole array, or a multi-dimensional
    /// one with its first dimensions indexed: a subarray) as element expressions from left to
    /// right (the order elements pair up in an array assignment, IEEE 1800-2017 7.6), and
    /// the array's signal and the sizes of the dimensions it spans (its shape). None for
    /// anything else.
    pub(crate) fn array_elems(&mut self, e: &Expr, sid: usize) -> R<Option<(u32, Vec<u32>, Vec<Expr>)>> {
        let Some((s, base, dims)) = self.array_dims(e, sid)? else { return Ok(None) };
        let mut elems = vec![base];
        for &d in &dims {
            elems = elems.into_iter().flat_map(|x| dim_indexes(d).map(move |v| index_expr(&x, v))).collect();
        }
        Ok(Some((s, dims.iter().map(|d| d.1).collect(), elems)))
    }

    /// The unpacked array, subarray or slice named by `e`: its signal, the expression its
    /// elements index (`e` itself, or the array a slice selects from) and the dimensions it
    /// spans, each (lowest index, size, descending).
    #[allow(clippy::type_complexity)]
    fn array_dims(&mut self, e: &Expr, sid: usize) -> R<Option<(u32, Expr, Vec<(i64, u32, bool)>)>> {
        if let Expr::Range(a, _, _) | Expr::IndexedRange(a, _, _, _) = e {
            if matches!(**a, Expr::Range(..) | Expr::IndexedRange(..)) {
                return Ok(None); // a slice of a slice
            }
            let Some((s, base, mut dims)) = self.array_dims(a, sid)? else { return Ok(None) };
            let desc = dims[0].2;
            let (left, right) = match e {
                Expr::Range(_, m, l) => (self.const_int(m, sid)?, self.const_int(l, sid)?),
                Expr::IndexedRange(_, b, w, up) => {
                    let (b, w) = (self.const_int(b, sid)?, self.const_int(w, sid)?);
                    let far = if *up { b + w - 1 } else { b - w + 1 };
                    if (far > b) == desc {
                        (far, b)
                    } else {
                        (b, far)
                    }
                }
                _ => unreachable!(),
            };
            let (lo, n) = (dims[0].0, dims[0].1 as i64);
            if left.min(right) < lo || left.max(right) >= lo + n {
                return self.err(format!("slice [{left}:{right}] of array '{}' is outside its range", self.sig(s).name));
            }
            if n > 1 && left != right && (left > right) != desc {
                return self.err(format!("slice [{left}:{right}] of array '{}' runs against its range", self.sig(s).name));
            }
            dims[0] = (left.min(right), ((left - right).abs() + 1) as u32, desc);
            return Ok(Some((s, base, dims)));
        }
        let mut nidx = 0;
        let mut base = e;
        while let Expr::Index(a, _) = base {
            nidx += 1;
            base = a;
        }
        if !matches!(base, Expr::Ident(_) | Expr::Hier(_)) {
            return Ok(None);
        }
        let Some(crate::expr::Base::Sig(s)) = self.resolve_base(base, sid)? else { return Ok(None) };
        let i = self.sig(s);
        if i.depth == 0 {
            return Ok(None);
        }
        let mut dims = match self.sigs.md_dims.get(&s) {
            Some(d) => d.clone(),
            None => vec![(i.arr_lo, i.depth, self.sigs.desc_arrays.contains(&s))],
        };
        if nidx >= dims.len() {
            return Ok(None);
        }
        dims.drain(..nidx);
        Ok(Some((s, e.clone(), dims)))
    }

    /// `lhs = '{...}` for an unpacked array (or subarray) indexed by `lhs` spanning `dims`: one
    /// assignment per element into `out` (IEEE 1800-2017 10.9.1: positional items left to
    /// right, index keys, `default:` for the rest; nested patterns for the inner dimensions).
    fn pattern_pairs(&mut self, lhs: &Expr, dims: &[(i64, u32, bool)], items: &[PatItem], sid: usize, out: &mut Vec<(Expr, Expr)>) -> R<()> {
        let idx: Vec<i64> = dim_indexes(dims[0]).collect();
        let mut vals: Vec<Option<&Expr>> = vec![None; idx.len()];
        let (mut npos, mut default) = (0, None);
        for it in items {
            let (k, v) = match it {
                PatItem::Pos(v) => {
                    if let Some(x) = vals.get_mut(npos) {
                        *x = Some(v);
                    }
                    npos += 1;
                    continue;
                }
                PatItem::Default(v) => {
                    default = Some(v);
                    continue;
                }
                PatItem::Key(k, v) => (self.const_int(&Expr::Ident(k.clone()), sid)?, v),
                PatItem::Index(k, v) => (self.const_int(k, sid)?, v),
            };
            let Some(p) = idx.iter().position(|&i| i == k) else {
                return self.err(format!("assignment pattern key {k} is outside the array's range"));
            };
            vals[p] = Some(v);
        }
        if npos > 0 && npos != idx.len() {
            return self.err(format!("assignment pattern has {npos} values for {} elements", idx.len()));
        }
        for (p, &i) in idx.iter().enumerate() {
            let el = index_expr(lhs, i);
            match (vals[p], default) {
                (Some(v), _) => self.elem_pairs(el, &dims[1..], v, false, sid, out)?,
                (None, Some(d)) => self.elem_pairs(el, &dims[1..], d, true, sid, out)?,
                (None, None) => return self.err(format!("assignment pattern has no value for element {i}")),
            }
        }
        Ok(())
    }

    /// The assignments of value `v` to element `el` of an array pattern, `el` spanning the
    /// unpacked dimensions `dims` (none: a single element). A `default:` value that is not
    /// itself an array goes to every element below `el`.
    fn elem_pairs(&mut self, el: Expr, dims: &[(i64, u32, bool)], v: &Expr, default: bool, sid: usize, out: &mut Vec<(Expr, Expr)>) -> R<()> {
        if dims.is_empty() {
            out.push((el, v.clone()));
            return Ok(());
        }
        if let Expr::Pattern(None, items) = v {
            return self.pattern_pairs(&el, dims, items, sid, out);
        }
        if default && self.array_dims(v, sid)?.is_none() {
            for i in dim_indexes(dims[0]) {
                self.elem_pairs(index_expr(&el, i), &dims[1..], v, true, sid, out)?;
            }
            return Ok(());
        }
        match self.array_pairs(&el, v, sid)? {
            Some(p) => out.extend(p),
            None => unreachable!("an element spanning dimensions is an array"),
        }
        Ok(())
    }

    /// An array shape and element width for messages: `[2][3] of 8 bits`.
    pub(crate) fn shape_str(shape: &[u32], w: u32) -> String {
        let dims: String = shape.iter().map(|n| format!("[{n}]")).collect();
        format!("{dims} of {w} bits")
    }

    /// Unpacked array `rs` assigned to `ls`: their element types must be equivalent (IEEE
    /// 1800-2017 6.22.2, 7.6).
    pub(crate) fn check_array_equiv(&self, ls: u32, rs: u32) -> R<()> {
        let (lc, rc) = (self.canon(ls), self.canon(rs));
        let why = if self.sigs.two_state.contains(&lc) != self.sigs.two_state.contains(&rc) {
            "one is 2-state, the other 4-state"
        } else if self.sig(ls).signed != self.sig(rs).signed {
            "their signedness differs"
        } else if self.enums.sigs.get(&ls) != self.enums.sigs.get(&rs) {
            "the enum types differ"
        } else {
            return Ok(());
        };
        self.err(format!("array '{}' assigned from '{}': the element types are not equivalent ({why})", self.sig(ls).name, self.sig(rs).name))
    }

    /// `lhs = rhs` of two unpacked arrays (or subarrays) as one assignment per element pair
    /// (None if `lhs` is not an array).
    pub(crate) fn array_pairs(&mut self, lhs: &Expr, rhs: &Expr, sid: usize) -> R<Option<Vec<(Expr, Expr)>>> {
        let Some((ls, lshape, li)) = self.array_elems(lhs, sid)? else { return Ok(None) };
        if let Expr::Pattern(None, items) = rhs {
            let (_, base, dims) = self.array_dims(lhs, sid)?.unwrap();
            let mut out = Vec::new();
            self.pattern_pairs(&base, &dims, items, sid, &mut out)?;
            return Ok(Some(out));
        }
        let Some((rs, rshape, ri)) = self.array_elems(rhs, sid)? else {
            return self.err(format!("array '{}' assigned from something that is not an array of the same shape", self.sig(ls).name));
        };
        let (lw, rw) = (self.sig(ls).width, self.sig(rs).width);
        if lshape != rshape || lw != rw {
            return self.err(format!(
                "array '{}' ({}) assigned from '{}' ({})",
                self.sig(ls).name,
                Self::shape_str(&lshape, lw),
                self.sig(rs).name,
                Self::shape_str(&rshape, rw)
            ));
        }
        self.check_array_equiv(ls, rs)?;
        Ok(Some(li.into_iter().zip(ri).collect()))
    }

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
        let jobs = self.alias_assigns(jobs)?;
        // nets with more than one continuous driver are resolved from per-driver signals
        let mut count: HashMap<u32, u32> = HashMap::new();
        for job in &jobs {
            for s in self.job_targets(job) {
                *count.entry(s).or_default() += 1;
            }
        }
        // variables a continuous assignment drives whole: no other driver (6.5)
        let mut whole: Vec<(u32, usize, u32)> = Vec::new();
        for job in &jobs {
            if let Job::Assign { lhs, scope, line, .. } | Job::PortOut { lhs, scope, line, .. } = job {
                let sym = match lhs {
                    Expr::Ident(n) => self.lookup(n, *scope),
                    Expr::Hier(p) => self.lookup_hier(p, *scope),
                    _ => None,
                };
                if let Some(Sym::Sig(s)) = sym {
                    let i = self.sig(s);
                    if i.kind != SigKind::Wire && i.depth == 0 {
                        whole.push((self.canon(s), *scope, *line));
                    }
                }
            }
        }
        for &(s, scope, line) in &whole {
            if count.get(&s).is_some_and(|&n| n > 1) {
                self.line = line;
                self.file = self.scopes[scope].file.clone();
                let n = self.sig(s).name.clone();
                return self.err(format!("variable '{n}' is driven by more than one continuous assignment"));
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
        for &(s, scope, line) in &whole {
            if self.code.proc_writes.contains(&s) {
                self.line = line;
                self.file = self.scopes[scope].file.clone();
                let n = self.sig(s).name.clone();
                return self.err(format!("variable '{n}' is written by both a continuous and a procedural assignment"));
            }
        }
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
                            // 9.2.2.2.2: it runs once at time 0 after all initial and always
                            // procedures have started: it waits first and gets a time-0 wake
                            let ctl = self.make_ctl(Vec::new());
                            let top = self.here();
                            self.emit(Instr::Wait { ctl });
                            self.stmt(body, *scope)?;
                            self.emit(Instr::Jmp { to: top });
                            // IEEE 1800 9.2.2.2.1: what the block and the functions it calls read,
                            // except what the block writes itself (after the body is compiled:
                            // the scopes of its blocks exist then)
                            let mut reads = Vec::new();
                            self.subr.func_reads.set(true);
                            self.reads_stmt(body, *scope, &mut reads);
                            self.subr.func_reads.set(false);
                            let mut written = Vec::new();
                            self.blocking_writes(body, *scope, &mut written);
                            reads.retain(|(s, _)| !written.contains(s));
                            let mut trig: Vec<(u32, rt::Edge, u32)> = reads.into_iter().map(|(s, e)| (s, rt::Edge::Any, e)).collect();
                            trig.sort_by_key(|t| (t.0, t.2, t.1 as u8));
                            trig.dedup();
                            self.prog.ctls[ctl as usize].triggers = trig;
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
                    self.check_enum_assign(lhs, rhs, *scope)?;
                    self.check_const_selects(lhs, *scope)?;
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

/// Memory ranges (start, words), sorted and with overlapping or adjacent ones merged.
pub(crate) fn merge_ranges(mut f: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    f.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (lo, n) in f {
        match merged.last_mut() {
            Some(m) if m.0 + m.1 >= lo => m.1 = m.1.max(lo + n - m.0),
            _ => merged.push((lo, n)),
        }
    }
    merged
}

/// The indexes of an unpacked dimension (lowest index, size, descending) from left to right.
fn dim_indexes((lo, n, desc): (i64, u32, bool)) -> impl Iterator<Item = i64> {
    let n = n as i64;
    (0..n).map(move |k| if desc { lo + n - 1 - k } else { lo + k })
}

/// `e[v]` with a constant index.
fn index_expr(e: &Expr, v: i64) -> Expr {
    let num = Expr::Num(ferrooxide_front::ast::dec(v.unsigned_abs()));
    let num = if v < 0 { Expr::Unary(ferrooxide_front::ast::UnOp::Neg, Box::new(num)) } else { num };
    Expr::Index(Box::new(e.clone()), Box::new(num))
}

impl<'a> Compiler<'a> {
    /// The selects in the target of a continuous assignment (or of a force or procedural
    /// assign) are constant (IEEE 1800-2017 10.3.1, 10.6: a net, a constant bit- or
    /// part-select of it, an element at a constant index).
    pub(crate) fn check_const_selects(&mut self, lhs: &Expr, sid: usize) -> R<()> {
        match lhs {
            Expr::Index(a, i) | Expr::IndexedRange(a, i, _, _) => {
                if !self.is_const_expr(i, sid) {
                    return self.err("a select in the target of a continuous assignment or force is not a constant expression");
                }
                self.check_const_selects(a, sid)
            }
            Expr::Range(a, ..) | Expr::Member(a, _) => self.check_const_selects(a, sid),
            Expr::Concat(v) => v.iter().try_for_each(|x| self.check_const_selects(x, sid)),
            _ => Ok(()),
        }
    }
}
