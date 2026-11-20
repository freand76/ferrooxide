//! Nets: gates, bidirectional links (tran islands), port aliases of continuous assignments.

use crate::*;

impl<'a> Compiler<'a> {
    /// A gate instance or an array of them (1-bit terminals per instance).
    pub(crate) fn gate_items(&mut self, kind: &str, terms: &'a [Expr], dims: &Option<Range>, sid: usize, line: u32) -> R<()> {
                    match dims {
                        None => self.gate(kind, terms, sid, line)?,
                        Some((l, r)) => {
                            // an array of gates: 1-bit terminals per instance
                            let (l, r) = (self.const_int(l, sid)?, self.const_int(r, sid)?);
                            let n = ((l - r).abs() + 1) as u32;
                            for k in 0..n {
                                let mut ts = Vec::new();
                                for t in terms {
                                    let tw = self.type_of(t, sid)?.width();
                                    ts.push(if tw == n && n > 1 { self.array_slice(t, k, n, 1, sid)? } else { t.clone() });
                                }
                                self.gate(kind, &ts, sid, line)?;
                            }
                        }
                    }
        Ok(())
    }

    /// Net bits named by `e` as (signal, lsb, width), most significant first; None if `e` is not
    /// made of constant selects of scalar nets.
    pub(crate) fn net_segs(&mut self, e: &Expr, sid: usize) -> Option<Vec<(u32, u32, u32)>> {
        let base = |me: &mut Self, b: &Expr| -> Option<u32> {
            match me.resolve_base(b, sid).ok()?? {
                expr::Base::Sig(s) => {
                    let i = me.sig(s);
                    (i.kind == SigKind::Wire && i.depth == 0 && !me.sig_type(s).is_real()).then_some(s)
                }
                _ => None,
            }
        };
        match e {
            Expr::Ident(_) | Expr::Hier(_) => {
                let s = base(self, e)?;
                Some(vec![(s, 0, self.sig(s).width)])
            }
            Expr::Index(b, i) => {
                let s = base(self, b)?;
                let i = self.const_value(i, sid)?;
                let info = self.sig(s);
                let (neg, k) = Self::bit_map(info.msb, info.lsb);
                let off = if neg { k - i } else { i + k };
                (off >= 0 && off < info.width as i64).then(|| vec![(s, off as u32, 1)])
            }
            Expr::Range(b, m, l) => {
                let s = base(self, b)?;
                let (m, l) = (self.const_value(m, sid)?, self.const_value(l, sid)?);
                let info = self.sig(s);
                let (neg, k) = Self::bit_map(info.msb, info.lsb);
                let (om, ol) = if neg { (k - m, k - l) } else { (m + k, l + k) };
                let (lo, hi) = (om.min(ol), om.max(ol));
                (lo >= 0 && hi < info.width as i64).then(|| vec![(s, lo as u32, (hi - lo + 1) as u32)])
            }
            Expr::Concat(v) => {
                let mut out = Vec::new();
                for x in v {
                    out.extend(self.net_segs(x, sid)?);
                }
                Some(out)
            }
            _ => None,
        }
    }

    /// Link the bits of `x` (in scope `xs`) and `y` (in `ys`) bidirectionally, LSB first, over the
    /// common width. False if either is not made of nets.
    pub(crate) fn link_exprs(&mut self, x: &Expr, xs: usize, y: &Expr, ys: usize, ctl: Option<(Expr, usize, bool)>) -> bool {
        let (Some(xa), Some(ya)) = (self.net_segs(x, xs), self.net_segs(y, ys)) else { return false };
        // expand into LSB-first runs and pair them up
        let (mut xi, mut yi) = (xa.into_iter().rev(), ya.into_iter().rev());
        let (mut xc, mut yc) = (xi.next(), yi.next());
        while let (Some((xsig, xl, xw)), Some((ysig, yl, yw))) = (xc, yc) {
            let w = xw.min(yw);
            self.nets.links.push(Link { a: xsig, a_lsb: xl, b: ysig, b_lsb: yl, w, ctl: ctl.clone() });
            xc = if xw > w { Some((xsig, xl + w, xw - w)) } else { xi.next() };
            yc = if yw > w { Some((ysig, yl + w, yw - w)) } else { yi.next() };
        }
        true
    }

    pub(crate) fn gate(&mut self, kind: &str, terms: &[Expr], sid: usize, line: u32) -> R<()> {
        if terms.len() < 2 && !matches!(kind, "pullup" | "pulldown") {
            return self.err("gate needs at least two terminals");
        }
        if kind.contains("tran") {
            // bidirectional switches (strengths are ignored: rtran = tran)
            let ctl = match kind.trim_start_matches('r') {
                "tran" if terms.len() == 2 => None,
                "tranif1" | "tranif0" if terms.len() == 3 => Some((terms[2].clone(), sid, kind.ends_with('1'))),
                _ => return self.err(format!("{kind}: wrong number of terminals")),
            };
            if !self.link_exprs(&terms[0], sid, &terms[1], sid, ctl) {
                return self.err(format!("{kind} terminals must be nets"));
            }
            return Ok(());
        }
        let b = |e: &Expr| Box::new(e.clone());
        let z = || Expr::Num(Number { size: None, signed: false, base: 2, based: true, digits: "z".into(), fill: true });
        match kind {
            "pullup" | "pulldown" => {
                // the pulled bits: (net, lsb, width, up)
                for t in terms {
                    match self.net_segs(t, sid) {
                        Some(segs) => {
                            for (s, lsb, w) in segs {
                                self.nets.pulls.entry(s).or_default().push((lsb, w, kind == "pullup"));
                            }
                        }
                        None => {
                            let mut v = Vec::new();
                            self.lvalue_bases(t, sid, &mut v);
                            for s in v {
                                let w = self.sig(s).width;
                                self.nets.pulls.entry(s).or_default().push((0, w, kind == "pullup"));
                            }
                        }
                    }
                }
            }
            "bufif0" | "bufif1" | "notif0" | "notif1" | "nmos" | "pmos" | "rnmos" | "rpmos" => {
                // (out, in, control): tri-state buffer; an x control merges the two choices
                if terms.len() != 3 {
                    return self.err(format!("{kind} needs three terminals"));
                }
                // bufif/notif turn a z input into x; the switches pass it through
                let data = if kind.starts_with("notif") {
                    Expr::Unary(UnOp::Not, b(&terms[1]))
                } else if kind.starts_with("bufif") {
                    Expr::Binary(BinOp::And, b(&terms[1]), b(&terms[1]))
                } else {
                    terms[1].clone()
                };
                let on_high = matches!(kind, "bufif1" | "notif1" | "nmos" | "rnmos");
                let rhs = if on_high {
                    Expr::Cond(b(&terms[2]), Box::new(data), Box::new(z()))
                } else {
                    Expr::Cond(b(&terms[2]), Box::new(z()), Box::new(data))
                };
                self.push_job(Job::Assign { lhs: terms[0].clone(), rhs, delay: self.nets.cur_delay.clone(), scope: sid, line });
            }
            "cmos" | "rcmos" => {
                // (out, in, ncontrol, pcontrol)
                if terms.len() != 4 {
                    return self.err(format!("{kind} needs four terminals"));
                }
                let p_side = Expr::Cond(b(&terms[3]), Box::new(z()), b(&terms[1]));
                let rhs = Expr::Cond(b(&terms[2]), b(&terms[1]), Box::new(p_side));
                self.push_job(Job::Assign { lhs: terms[0].clone(), rhs, delay: self.nets.cur_delay.clone(), scope: sid, line });
            }
            "not" | "buf" => {
                let input = terms.last().unwrap();
                for out in &terms[..terms.len() - 1] {
                    // a buffer turns z into x: `in & in`
                    let rhs = if kind == "not" {
                        Expr::Unary(UnOp::Not, b(input))
                    } else {
                        Expr::Binary(BinOp::And, b(input), b(input))
                    };
                    self.push_job(Job::Assign { lhs: out.clone(), rhs, delay: self.nets.cur_delay.clone(), scope: sid, line });
                }
            }
            _ => {
                let (op, inv) = match kind {
                    "and" => (BinOp::And, false),
                    "nand" => (BinOp::And, true),
                    "or" => (BinOp::Or, false),
                    "nor" => (BinOp::Or, true),
                    "xor" => (BinOp::Xor, false),
                    _ => (BinOp::Xor, true),
                };
                let mut e = terms[1].clone();
                for t in &terms[2..] {
                    e = Expr::Binary(op, Box::new(e), b(t));
                }
                if inv {
                    e = Expr::Unary(UnOp::Not, Box::new(e));
                }
                self.push_job(Job::Assign { lhs: terms[0].clone(), rhs: e, delay: self.nets.cur_delay.clone(), scope: sid, line });
            }
        }
        Ok(())
    }

    /// Names that some `force` / procedural `assign` targets (the last component of the
    /// target's name), in all modules.
    pub(crate) fn forced_names(&self) -> HashSet<String> {
        fn expr_name(e: &Expr, out: &mut HashSet<String>) {
            match e {
                Expr::Ident(n) => {
                    out.insert(n.clone());
                }
                Expr::Hier(p) => {
                    if let Some(n) = p.last() {
                        out.insert(n.clone());
                    }
                }
                Expr::Index(a, _) | Expr::Range(a, _, _) | Expr::IndexedRange(a, _, _, _) => expr_name(a, out),
                Expr::Concat(v) => v.iter().for_each(|x| expr_name(x, out)),
                _ => {}
            }
        }
        fn stmt(s: &Stmt, out: &mut HashSet<String>) {
            match s {
                Stmt::Force { lhs, .. } => expr_name(lhs, out),
                Stmt::Block { stmts, .. } => stmts.iter().for_each(|x| stmt(x, out)),
                Stmt::If { then_s, else_s, .. } => {
                    stmt(then_s, out);
                    if let Some(e) = else_s {
                        stmt(e, out);
                    }
                }
                Stmt::Case { arms, default, .. } => {
                    arms.iter().for_each(|a| stmt(&a.1, out));
                    if let Some(d) = default {
                        stmt(d, out);
                    }
                }
                Stmt::For { init, step, body, .. } => {
                    stmt(init, out);
                    stmt(step, out);
                    stmt(body, out);
                }
                Stmt::While { body, .. }
                | Stmt::DoWhile { body, .. }
                | Stmt::Repeat { body, .. }
                | Stmt::Forever { body, .. }
                | Stmt::Timed { body, .. }
                | Stmt::Wait { body, .. } => stmt(body, out),
                _ => {}
            }
        }
        fn items(its: &[Item], out: &mut HashSet<String>) {
            for it in its {
                match it {
                    Item::Process { body, .. } => stmt(body, out),
                    Item::Subroutine(f) => stmt(&f.body, out),
                    Item::GenFor { items: i, .. } | Item::GenBlock { items: i, .. } => items(i, out),
                    Item::GenIf { then_items, else_items, .. } => {
                        items(then_items, out);
                        items(else_items, out);
                    }
                    Item::GenCase { arms, .. } => arms.iter().for_each(|a| items(&a.2, out)),
                    _ => {}
                }
            }
        }
        let mut out = HashSet::new();
        fn module(m: &Module, out: &mut HashSet<String>) {
            items(&m.items, out);
            m.nested.iter().for_each(|n| module(n, out));
        }
        for m in self.hier.mods.values() {
            module(m, &mut out);
        }
        out
    }

    /// A plain continuous assignment of a whole signal to a net of the same width (`assign z =
    /// a;`, `wire z = a;`, also `{a}`, a constant-index array element, a zero delay) makes the
    /// net another name for the source, as in the reference: `z` changes together with `a`,
    /// inside the process that writes `a`. Not for nets with other drivers, special net types,
    /// strengths, links or forces (a forced alias would force its source). The same for a
    /// whole unpacked array assigned from one of the same shape; other whole-array
    /// assignments become one assignment per element.
    pub(crate) fn alias_assigns(&mut self, jobs: Vec<Job<'a>>) -> R<Vec<Job<'a>>> {
        let mut count: HashMap<u32, u32> = HashMap::new();
        for job in &jobs {
            for s in self.job_targets(job) {
                *count.entry(s).or_default() += 1;
            }
        }
        let linked: HashSet<u32> = self.nets.links.iter().flat_map(|l| [self.canon(l.a), self.canon(l.b)]).collect();
        let fnames = self.forced_names();
        let mut forced: HashSet<u32> = HashSet::new();
        if !fnames.is_empty() {
            for sc in &self.scopes {
                for (n, sym) in &sc.syms {
                    if let (true, Sym::Sig(s)) = (fnames.contains(n), sym) {
                        forced.insert(self.canon(*s));
                    }
                }
            }
        }
        let old_strength = std::mem::take(&mut self.code.job_strength);
        let mut kept = Vec::with_capacity(jobs.len());
        let mut changed = false;
        for (k, job) in jobs.into_iter().enumerate() {
            if !old_strength.contains_key(&k) {
                if let Job::Assign { lhs, rhs, delay, scope, line } = &job {
                    if let Some((z, a)) = self.alias_pair(lhs, rhs, delay.as_ref(), *scope) {
                        if self.sig(z).depth > 0 && self.sig(a).depth > 0 {
                            self.line = *line;
                            self.file = self.scopes[*scope].file.clone();
                            self.check_array_equiv(z, a)?;
                        }
                        let i = self.sig(z);
                        let ok = count.get(&z) == Some(&1)
                            && i.kind != SigKind::Integer
                            && (i.depth == 0 || !self.sigs.md_dims.contains_key(&z))
                            && i.alias_of.is_none()
                            && !self.sig_type(z).is_real()
                            && !self.nets.net_types.contains_key(&z)
                            && !self.nets.pulls.contains_key(&z)
                            && !self.sigs.two_state.contains(&z)
                            && !linked.contains(&z)
                            && !forced.contains(&z);
                        if ok {
                            if i.kind == SigKind::Reg {
                                self.nets.alias_vars.insert(z);
                            }
                            self.nets.view_of.insert(z, a);
                            changed = true;
                            continue;
                        }
                    }
                }
            }
            if let Job::Assign { lhs, rhs, delay, scope, line } = &job {
                self.line = *line;
                self.file = self.scopes[*scope].file.clone();
                if let Some(pairs) = self.array_pairs(lhs, rhs, *scope)? {
                    for (l, r) in pairs {
                        if let Some(st) = old_strength.get(&k) {
                            self.code.job_strength.insert(kept.len(), *st);
                        }
                        kept.push(Job::Assign { lhs: l, rhs: r, delay: delay.clone(), scope: *scope, line: *line });
                    }
                    continue;
                }
            }
            if let Some(st) = old_strength.get(&k) {
                self.code.job_strength.insert(kept.len(), *st);
            }
            kept.push(job);
        }
        if changed {
            // views share the storage of the signal they stand for
            let views: Vec<u32> = self.nets.view_of.keys().copied().collect();
            for v in views {
                let b = self.canon(v);
                self.prog.signals[v as usize].slot = self.prog.signals[b as usize].slot;
            }
        }
        Ok(kept)
    }

    /// For `assign z = a` with a whole-signal source of the same width: (net, source), both
    /// as the signals they stand for.
    pub(crate) fn alias_pair(&mut self, lhs: &Expr, rhs: &Expr, delay: Option<&Expr>, sid: usize) -> Option<(u32, u32)> {
        if let Some(d) = delay {
            if self.const_value(d, sid) != Some(0) {
                return None;
            }
        }
        if !matches!(lhs, Expr::Ident(_) | Expr::Hier(_)) {
            return None;
        }
        let Ok(Some(expr::Base::Sig(z))) = self.resolve_base(lhs, sid) else { return None };
        let z = self.canon(z);
        let mut rhs = rhs;
        while let Expr::Concat(v) = rhs {
            if v.len() != 1 {
                return None;
            }
            rhs = &v[0];
        }
        let width = self.sig(z).width;
        let zdepth = self.sig(z).depth;
        let shape = |c: &Self, s: u32| (c.sig(s).depth, c.sig(s).arr_lo, c.sigs.desc_arrays.contains(&s), c.sigs.md_dims.contains_key(&s));
        let a = match rhs {
            // (a whole array: from one of the same shape)
            Expr::Ident(_) | Expr::Hier(_) => match self.resolve_base(rhs, sid) {
                Ok(Some(expr::Base::Sig(a))) if self.sig(a).depth == zdepth && (zdepth == 0 || shape(self, a) == shape(self, z)) => a,
                _ => return None,
            },
            Expr::Index(..) if zdepth == 0 => {
                let name = format!("{}$alias", self.sig(z).name);
                let a = self.elem_port_alias(rhs, sid, width, &name)?;
                self.prog.names.retain(|n| n.1 != a);
                a
            }
            _ => return None,
        };
        let a = self.canon(a);
        let ai = self.sig(a);
        if a == z || ai.width != width || self.sig_type(a).is_real() {
            return None;
        }
        Some((z, a))
    }

    /// What undriven (z) bits of a net read as: 1 (pullup, tri1), 0 (pulldown, tri0) or z.
    pub(crate) fn pull_const(&self, s: u32) -> Option<Const> {
        let w = self.sig(s).width;
        let mut c = match self.nets.net_types.get(&s) {
            Some(NetType::Tri0) => Const::fill('0', w),
            Some(NetType::Tri1) => Const::fill('1', w),
            _ if self.nets.pulls.contains_key(&s) => Const::fill('z', w),
            _ => return None,
        };
        for &(lsb, pw, up) in self.nets.pulls.get(&s).map(|v| v.as_slice()).unwrap_or(&[]) {
            for b in lsb..(lsb + pw).min(w) {
                let (i, m) = ((b / 64) as usize, 1u64 << (b % 64));
                c.b[i] &= !m;
                if up {
                    c.a[i] |= m;
                } else {
                    c.a[i] &= !m;
                }
            }
        }
        Some(c)
    }

    /// Linked nets form islands (connected through links). One process per island computes all
    /// its nets at once: each net starts from its own drivers, then the links exchange values
    /// (resolve the two sides; a tranif with an unknown control blends) until every path is
    /// covered. Solving the island in one activation (as Icarus does) avoids the oscillation of
    /// separate per-link processes in loops. Returns the island nets (they get no resolver).
    pub(crate) fn link_islands(&mut self) -> R<HashSet<u32>> {
        let links = std::mem::take(&mut self.nets.links);
        let mut nets_done = HashSet::new();
        if links.is_empty() {
            return Ok(nets_done);
        }
        // union-find over the linked nets
        let mut parent: HashMap<u32, u32> = HashMap::new();
        fn find(p: &mut HashMap<u32, u32>, x: u32) -> u32 {
            let mut r = x;
            while let Some(&q) = p.get(&r) {
                if q == r {
                    break;
                }
                r = q;
            }
            p.insert(x, r);
            r
        }
        for l in &links {
            for s in [l.a, l.b] {
                parent.entry(s).or_insert(s);
            }
            let (ra, rb) = (find(&mut parent, l.a), find(&mut parent, l.b));
            if ra != rb {
                parent.insert(ra, rb);
            }
        }
        let mut islands: Vec<(u32, Vec<usize>)> = Vec::new();
        for (k, l) in links.iter().enumerate() {
            let r = find(&mut parent, l.a);
            match islands.iter_mut().find(|(x, _)| *x == r) {
                Some((_, v)) => v.push(k),
                None => islands.push((r, vec![k])),
            }
        }
        for (_, ks) in islands {
            let mut nets: Vec<u32> = Vec::new();
            for &k in &ks {
                for s in [links[k].a, links[k].b] {
                    if !nets.contains(&s) {
                        nets.push(s);
                    }
                }
            }
            let name = self.sig(nets[0]).name.clone();
            self.begin_proc(name, ProcClass::Normal);
            let top = self.here();
            let mut trig: Vec<(u32, rt::Edge, u32)> = Vec::new();
            // controls, evaluated once per activation; a control that is a net of the island
            // itself takes its value from the relaxation (starting from its own drivers)
            let mut ctls: Vec<Option<(u32, u32, bool)>> = Vec::new();
            let mut ctl_net: Vec<Option<u32>> = Vec::new();
            for &k in &ks {
                let own = match &links[k].ctl {
                    Some((ce @ (Expr::Ident(_) | Expr::Hier(_)), cs, _)) => match self.resolve_base(ce, *cs)? {
                        Some(expr::Base::Sig(s)) if nets.contains(&s) => Some(s),
                        _ => None,
                    },
                    _ => None,
                };
                ctl_net.push(own);
                if let (Some(s), Some((_, _, on1))) = (own, &links[k].ctl) {
                    ctls.push(Some((s, self.sig(s).width, *on1)));
                    continue;
                }
                ctls.push(match &links[k].ctl {
                    Some((ce, cs, on1)) => {
                        let (c, cw, _) = self.self_expr(ce, *cs)?;
                        let keep = self.tmp(cw);
                        self.emit(Instr::Copy { d: keep, s: c, w: cw });
                        let mut r = Vec::new();
                        self.reads_expr(ce, *cs, &mut r);
                        trig.extend(r.into_iter().map(|(s, e)| (s, rt::Edge::Any, e)));
                        Some((keep, cw, *on1))
                    }
                    None => None,
                });
            }
            // each net from its own drivers
            let mut cur: HashMap<u32, u32> = HashMap::new();
            for &n in &nets {
                let w = self.sig(n).width;
                let drv = self.nets.drivers.get(&n).cloned().unwrap_or_default();
                let v = self.tmp(w);
                if drv.is_empty() {
                    let z = self.const_slot(&Const::fill('z', w));
                    self.emit(Instr::Copy { d: v, s: z, w });
                } else {
                    let first = self.sig(drv[0]).slot;
                    self.emit(Instr::Copy { d: v, s: first, w });
                    for &d in &drv[1..] {
                        let ds = self.sig(d).slot;
                        self.emit(Instr::Bin { op: rt::BinOp::Resolve, d: v, a: v, b: ds, w });
                    }
                }
                trig.extend(drv.iter().map(|&d| (d, rt::Edge::Any, NONE)));
                cur.insert(n, v);
            }
            // relax over the links: one pass per link covers every path
            for _ in 0..ks.len() {
                for (j, &k) in ks.iter().enumerate() {
                    let l = links[k].clone();
                    let (ca, cb) = (cur[&l.a], cur[&l.b]);
                    let (wa, wb) = (self.sig(l.a).width, self.sig(l.b).width);
                    let ta = self.tmp(l.w);
                    let tb = self.tmp(l.w);
                    self.emit(Instr::Slice { d: ta, s: ca, sw: wa, lsb: l.a_lsb, w: l.w });
                    self.emit(Instr::Slice { d: tb, s: cb, sw: wb, lsb: l.b_lsb, w: l.w });
                    let r = self.tmp(l.w);
                    self.emit(Instr::Bin { op: rt::BinOp::Resolve, d: r, a: ta, b: tb, w: l.w });
                    let (ra, rb) = match ctls[j] {
                        None => (r, r),
                        Some((c, cw, on1)) => {
                            let c = match ctl_net[j] {
                                Some(n) => cur[&n],
                                None => c,
                            };
                            let (xa, xb) = (self.tmp(l.w), self.tmp(l.w));
                            let (ta2, tb2) = if on1 { ((r, ta), (r, tb)) } else { ((ta, r), (tb, r)) };
                            self.emit(Instr::Mux { d: xa, c, cw, t: ta2.0, f: ta2.1, w: l.w });
                            self.emit(Instr::Mux { d: xb, c, cw, t: tb2.0, f: tb2.1, w: l.w });
                            (xa, xb)
                        }
                    };
                    self.emit(Instr::Insert { d: ca, dw: wa, s: ra, sw: l.w, off: l.a_lsb });
                    self.emit(Instr::Insert { d: cb, dw: wb, s: rb, sw: l.w, off: l.b_lsb });
                }
            }
            for &n in &nets {
                let w = self.sig(n).width;
                let mut v = cur[&n];
                if let Some(c) = self.pull_const(n) {
                    let c = self.const_slot(&c);
                    let t = self.tmp(w);
                    self.emit(Instr::Bin { op: rt::BinOp::PullZ, d: t, a: v, b: c, w });
                    v = t;
                }
                self.emit(Instr::Store { t: rt::Target::Whole { sig: n }, s: v, mode: rt::StoreMode::Blocking });
                nets_done.insert(n);
            }
            if trig.is_empty() {
                self.emit(Instr::Halt);
            } else {
                let ctl = self.make_ctl(trig);
                self.emit(Instr::Wait { ctl });
                self.emit(Instr::Jmp { to: top });
            }
            self.end_proc();
        }
        Ok(nets_done)
    }
}
