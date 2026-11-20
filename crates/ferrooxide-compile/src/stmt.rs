//! Statement code generation.

use crate::types::Type;
use crate::*;
use ferrooxide_rt::{CmpOp, Edge as REdge, FmtArg, StoreMode, SysTask, Target};

pub(crate) enum Lhs<'e> {
    Expr(&'e Expr, usize),
    Sig(u32),
}

pub(crate) enum Rhs<'e> {
    Expr(&'e Expr, usize),
    Sig(u32),
}

impl<'a> Compiler<'a> {
    /// Value of an assignment RHS converted to the LHS width.
    pub(crate) fn rhs_value(&mut self, rhs: &Expr, sid: usize, lw: u32) -> R<u32> {
        let (rw, rs) = self.type_of(rhs, sid)?.ws();
        let w = rw.max(lw);
        let v = self.expr(rhs, sid, w, rs)?;
        Ok(self.resize(v, w, lw, false))
    }

    /// Store targets of an lvalue, msb-first, with widths.
    fn targets(&mut self, e: &Expr, sid: usize, out: &mut Vec<(Option<Target>, u32)>) -> R<()> {
        if let Some(n) = self.lv_norm(e, sid) {
            return self.targets(&n, sid, out);
        }
        match e {
            Expr::Concat(v) => {
                for x in v {
                    self.targets(x, sid, out)?;
                }
                Ok(())
            }
            Expr::Ident(_) | Expr::Hier(_) => match self.resolve_base(e, sid)? {
                Some(expr::Base::Sig(s)) => {
                    let s = self.redir(s);
                    let i = self.sig(s);
                    if i.depth > 0 {
                        return self.err(format!("cannot assign to whole array '{}'", i.name));
                    }
                    out.push((Some(Target::Whole { sig: s }), i.width));
                    Ok(())
                }
                _ => self.err("assignment to a non-variable"),
            },
            Expr::Index(a, i) => {
                if let Some(arr) = self.array_of(a, sid) {
                    let info = self.sig(arr).clone();
                    if self.is_const(i, sid) {
                        let e = self.const_index(i, sid)?.map_or(-1, |v| v - info.arr_lo);
                        let t = if e >= 0 && e < info.depth as i64 {
                            Some(Target::ElemK { sig: arr, e: e as u32 })
                        } else {
                            None
                        };
                        out.push((t, info.width));
                        return Ok(());
                    }
                    let ix = self.index_of(i, sid, false, -info.arr_lo)?;
                    out.push((Some(Target::Elem { sig: arr, ix }), info.width));
                    return Ok(());
                }
                // bit of an array element: mem[i][b]
                if let Expr::Index(arr_e, ai) = &**a {
                    if let Some(arr) = self.array_of(arr_e, sid) {
                        let info = self.sig(arr).clone();
                        let ix = self.index_of(ai, sid, false, -info.arr_lo)?;
                        let (neg, k) = Self::bit_map(info.msb, info.lsb);
                        if !self.is_const(i, sid) {
                            // a bit at a run-time offset
                            let bix = self.index_of(i, sid, neg, k)?;
                            out.push((Some(Target::ElemDynPart { sig: arr, ix, bix, w: 1 }), 1));
                            return Ok(());
                        }
                        let off = match self.const_index(i, sid)? {
                            Some(b) if neg => k - b,
                            Some(b) => b + k,
                            None => -1,
                        };
                        if off < 0 || off >= info.width as i64 {
                            out.push((None, 1));
                        } else {
                            out.push((Some(Target::ElemPart { sig: arr, ix, lsb: off as u32, w: 1 }), 1));
                        }
                        return Ok(());
                    }
                }
                let s = self.lsig(a, sid)?;
                let info = self.sig(s).clone();
                let (neg, k) = Self::bit_map(info.msb, info.lsb);
                if self.is_const(i, sid) {
                    let off = match self.const_index(i, sid)? {
                        Some(b) if neg => k - b,
                        Some(b) => b + k,
                        None => -1,
                    };
                    if off < 0 || off >= info.width as i64 {
                        out.push((None, 1));
                    } else {
                        out.push((Some(Target::Part { sig: s, lsb: off as u32, w: 1 }), 1));
                    }
                } else {
                    let ix = self.index_of(i, sid, neg, k)?;
                    out.push((Some(Target::DynPart { sig: s, ix, w: 1 }), 1));
                }
                Ok(())
            }
            Expr::Range(a, m, l) => {
                let (Some(mv), Some(lv)) = (self.select_bound(m, sid)?, self.select_bound(l, sid)?) else {
                    // an x/z bound: nothing is written
                    out.push((None, 1));
                    return Ok(());
                };
                let pw = ((mv - lv).abs() + 1) as u32;
                if let Expr::Index(arr_e, ai) = &**a {
                    if let Some(arr) = self.array_of(arr_e, sid) {
                        let info = self.sig(arr).clone();
                        let ix = self.index_of(ai, sid, false, -info.arr_lo)?;
                        let (neg, k) = Self::bit_map(info.msb, info.lsb);
                        let off = if neg { k - lv } else { lv + k };
                        // only the bits inside the element are written
                        let (lo, hi) = (off.max(0), (off + pw as i64).min(info.width as i64));
                        if lo >= hi {
                            out.push((None, pw));
                            return Ok(());
                        }
                        if off + pw as i64 > hi {
                            out.push((None, (off + pw as i64 - hi) as u32));
                        }
                        out.push((Some(Target::ElemPart { sig: arr, ix, lsb: lo as u32, w: (hi - lo) as u32 }), (hi - lo) as u32));
                        if lo > off {
                            out.push((None, (lo - off) as u32));
                        }
                        return Ok(());
                    }
                }
                let s = self.lsig(a, sid)?;
                let info = self.sig(s).clone();
                let (neg, k) = Self::bit_map(info.msb, info.lsb);
                let off = if neg { k - lv } else { lv + k };
                if off >= 0 && off + pw as i64 <= info.width as i64 {
                    out.push((Some(Target::Part { sig: s, lsb: off as u32, w: pw }), pw));
                } else {
                    let c = self.const_slot(&Const::from_i64(off, 64, true));
                    let ix = rt::Index { slot: c, w: 64, signed: true, neg: false, k: 0 };
                    out.push((Some(Target::DynPart { sig: s, ix, w: pw }), pw));
                }
                Ok(())
            }
            Expr::IndexedRange(a, base, width, up) => {
                if !self.is_const_expr(width, sid) {
                    return self.err("the width of an indexed part-select is not a constant expression");
                }
                let pw = self.const_int(width, sid)? as u32;
                // of an array element: mem[i][b +: w]
                if let Expr::Index(arr_e, ai) = &**a {
                    if let Some(arr) = self.array_of(arr_e, sid) {
                        let info = self.sig(arr).clone();
                        let ix = self.index_of(ai, sid, false, -info.arr_lo)?;
                        let (neg, k) = Self::index_part_map(info.msb, info.lsb, pw, *up);
                        let bix = self.index_of(base, sid, neg, k)?;
                        out.push((Some(Target::ElemDynPart { sig: arr, ix, bix, w: pw }), pw));
                        return Ok(());
                    }
                }
                let s = self.lsig(a, sid)?;
                let info = self.sig(s).clone();
                let (neg, k) = Self::index_part_map(info.msb, info.lsb, pw, *up);
                let ix = self.index_of(base, sid, neg, k)?;
                out.push((Some(Target::DynPart { sig: s, ix, w: pw }), pw));
                Ok(())
            }
            Expr::Member(..) => self.member_error(e, sid),
            _ => self.err("invalid assignment target"),
        }
    }

    fn lsig(&mut self, e: &Expr, sid: usize) -> R<u32> {
        match self.resolve_base(e, sid)? {
            Some(expr::Base::Sig(s)) => Ok(self.redir(s)),
            _ => self.err("invalid assignment target"),
        }
    }

    fn lhs_targets(&mut self, lhs: &Lhs) -> R<Vec<(Option<Target>, u32)>> {
        match lhs {
            Lhs::Sig(s) => {
                let s = self.redir(*s);
                Ok(vec![(Some(Target::Whole { sig: s }), self.sig(s).width)])
            }
            Lhs::Expr(e, sid) => {
                let mut v = Vec::new();
                self.targets(e, *sid, &mut v)?;
                Ok(v)
            }
        }
    }

    pub(crate) fn lhs_width(&mut self, lhs: &Lhs) -> R<u32> {
        match lhs {
            Lhs::Sig(s) => Ok(self.sig(*s).width),
            Lhs::Expr(e, sid) => Ok(self.type_of(e, *sid)?.width()),
        }
    }

    /// Store a value (slot, width) into an lvalue.
    pub(crate) fn store_value(&mut self, lhs: &Lhs, slot: u32, w: u32, mode: StoreMode) -> R<()> {
        // an element of a queue or dynamic array
        if let Lhs::Expr(e @ Expr::Index(a, _), s) = lhs {
            if self.sigs.has_dyn {
                if let Some((_, elem)) = self.dyn_arr_type(a, *s)? {
                    let v = if elem.leaf == crate::types::Leaf::Bits && elem.class == NONE { self.resize(slot, w, elem.w, false) } else { slot };
                    if self.dyn_elem_store(e, v, *s)? {
                        return Ok(());
                    }
                }
            }
        }
        let ts = self.lhs_targets(lhs)?;
        let total: u32 = ts.iter().map(|t| t.1).sum();
        let v = self.resize(slot, w, total, false);
        if ts.len() == 1 {
            if let Some(t) = ts[0].0 {
                self.emit(Instr::Store { t, s: v, mode });
            }
            return Ok(());
        }
        let mut off = 0;
        for (t, tw) in ts.iter().rev() {
            if let Some(t) = t {
                let d = self.tmp(*tw);
                self.emit(Instr::Slice { d, s: v, sw: total, lsb: off, w: *tw });
                self.emit(Instr::Store { t: *t, s: d, mode });
            }
            off += tw;
        }
        Ok(())
    }

    /// A process for a continuous assignment / port connection.
    pub(crate) fn assign_proc(&mut self, lhs: Lhs, rhs: Rhs, delay: Option<&Expr>) -> R<()> {
        let name = match (&lhs, &rhs) {
            (Lhs::Expr(_, s), _) | (_, Rhs::Expr(_, s)) => self.scopes[*s].name.clone(),
            _ => String::new(),
        };
        let p = self.begin_proc(name, ProcClass::Normal);
        self.prog.procs[p as usize].fusible = delay.is_none();
        let top = self.here();
        let lw = self.lhs_width(&lhs)?;
        let mut reads = Vec::new();
        let v = match &rhs {
            Rhs::Expr(e, sid) => {
                let (real, layout) = match &lhs {
                    Lhs::Expr(l, ls) => (self.type_of(l, *ls)?, self.target_layout(l, *ls)),
                    Lhs::Sig(s) => (self.sig_type(*s), self.layout(*s).map(|l| (*l).clone())),
                };
                let v = self.value_for(e, *sid, lw, real, layout.as_ref())?;
                self.reads_expr(e, *sid, &mut reads);
                v
            }
            Rhs::Sig(s) => {
                reads.push((*s, NONE));
                let i = self.sig(*s).clone();
                self.resize(i.slot, i.width, lw, i.signed)
            }
        };
        if let Lhs::Expr(e, sid) = &lhs {
            self.reads_lvalue(e, *sid, &mut reads);
        }
        if let Some(d) = delay {
            let sid = match (&lhs, &rhs) {
                (Lhs::Expr(_, s), _) | (_, Rhs::Expr(_, s)) => *s,
                _ => 0,
            };
            // an inertial delay: the update is scheduled (replacing a pending one), the process
            // goes on waiting for its inputs
            let ds = self.delays3(d, sid)?;
            self.store_value(&lhs, v, lw, StoreMode::Inertial(ds))?;
            // the driven net reads x until the first delayed update (as in Icarus)
            for (t, _) in self.lhs_targets(&lhs)? {
                if let Some(Target::Whole { sig }) = t {
                    let i = self.sig(sig).clone();
                    if i.kind == SigKind::Wire && i.depth == 0 {
                        let n = rt::nw(i.width);
                        for k in 0..n {
                            let m = if k == n - 1 { rt::tmask(i.width) } else { !0 };
                            self.prog.mem[i.slot as usize + k] = m;
                            self.prog.mem[i.slot as usize + n + k] = m;
                        }
                    }
                }
            }
        } else {
            self.store_value(&lhs, v, lw, StoreMode::Blocking)?;
        }
        if reads.is_empty() {
            self.emit(Instr::Halt);
        } else {
            let ctl = self.make_ctl(reads.into_iter().map(|(s, e)| (s, REdge::Any, e)).collect());
            self.emit(Instr::Wait { ctl });
            self.emit(Instr::Jmp { to: top });
        }
        self.end_proc();
        Ok(())
    }

    /// A disable target resolved after code generation (first candidate path with blocks).
    pub(crate) fn disable_target(&mut self, cands: Vec<String>) -> u32 {
        let target = self.prog.disables.len() as u32;
        self.prog.disables.push(Vec::new());
        self.code.pending_disables.push((target, cands, self.file.clone(), self.line));
        target
    }

    /// `lhs` with its run-time indices replaced by hidden variables set now (their memory is
    /// added to `frame`): the target of an event-controlled nonblocking assignment is the one
    /// its indices name at the statement.
    fn capture_indices(&mut self, lhs: &Expr, sid: usize, frame: &mut Vec<(u32, u32)>) -> R<Expr> {
        let cap = |c: &mut Self, e: &Expr, frame: &mut Vec<(u32, u32)>| -> R<Box<Expr>> {
            if c.is_const(e, sid) {
                return Ok(Box::new(e.clone()));
            }
            let (v, w, signed) = c.self_expr(e, sid)?;
            let name = format!("$nbaix{}", c.prog.signals.len());
            let h = c.new_signal(format!("{}.{name}", c.scopes[sid].name), w, signed, 0, w as i64 - 1, 0, 0, SigKind::Reg);
            c.prog.names.pop(); // (no VCD entry)
            c.scopes[sid].syms.insert(name.clone(), Sym::Sig(h));
            let slot = c.sig(h).slot;
            c.emit(Instr::Copy { d: slot, s: v, w });
            frame.push((slot, rt::words(w)));
            Ok(Box::new(Expr::Ident(name)))
        };
        Ok(match lhs {
            Expr::Index(a, i) => {
                let a = self.capture_indices(a, sid, frame)?;
                Expr::Index(Box::new(a), cap(self, i, frame)?)
            }
            Expr::Range(a, m, l) => Expr::Range(Box::new(self.capture_indices(a, sid, frame)?), m.clone(), l.clone()),
            Expr::IndexedRange(a, b, w, up) => {
                let a = self.capture_indices(a, sid, frame)?;
                Expr::IndexedRange(Box::new(a), cap(self, b, frame)?, w.clone(), *up)
            }
            Expr::Concat(v) => Expr::Concat(v.iter().map(|x| self.capture_indices(x, sid, frame)).collect::<R<_>>()?),
            e => e.clone(),
        })
    }

    /// The control of an event control, and its expressions that hidden signals follow (with
    /// those signals).
    fn event_ctl<'e>(&mut self, ec: &'e EventCtl, body: &Stmt, sid: usize) -> R<(u32, Vec<(u32, &'e Expr)>)> {
        let mut trig = Vec::new();
        let mut followed = Vec::new();
        match ec {
            EventCtl::Star => {
                let mut reads = Vec::new();
                self.reads_stmt(body, sid, &mut reads);
                trig.extend(reads.into_iter().map(|(s, e)| (s, REdge::Any, e)));
            }
            EventCtl::List(v) => {
                for (edge, e) in v {
                    let re = match edge {
                        Edge::Any => REdge::Any,
                        Edge::Pos => REdge::Pos,
                        Edge::Neg => REdge::Neg,
                    };
                    // (IEEE 1800-2017 9.4.2: edges are of integral values)
                    if re != REdge::Any && self.type_of(e, sid)?.is_real() {
                        return self.err("an edge of a real value");
                    }
                    match e {
                        Expr::Ident(_) | Expr::Hier(_) => match self.resolve_base(e, sid)? {
                            Some(expr::Base::Sig(s)) => trig.push((s, re, NONE)),
                            _ => return self.err("event expression is not a signal"),
                        },
                        _ => {
                            // the value of the expression as a hidden signal, kept up to date by
                            // a combinational process; edges and changes are detected on it
                            let h = self.event_signal(e, sid)?;
                            trig.push((h, re, NONE));
                            followed.push((h, e));
                        }
                    }
                }
            }
        }
        Ok((self.make_ctl(trig), followed))
    }

    /// A hidden signal that follows the value of an event expression (e.g. `posedge a[3]`).
    fn event_signal(&mut self, e: &Expr, sid: usize) -> R<u32> {
        let (w, _) = self.type_of(e, sid)?.ws();
        let name = format!("{}.$event{}", self.scopes[sid].name, self.prog.signals.len());
        let h = self.new_signal(name, w, false, 0, w as i64 - 1, 0, 0, SigKind::Reg);
        self.prog.names.pop(); // not a user-visible name (no VCD entry)
        let p = self.begin_proc(self.scopes[sid].name.clone(), ProcClass::Normal);
        self.prog.procs[p as usize].fusible = true;
        let top = self.here();
        let v = self.expr(e, sid, w, false)?;
        self.emit(Instr::Store { t: Target::Whole { sig: h }, s: v, mode: StoreMode::Blocking });
        let mut reads = Vec::new();
        self.reads_expr(e, sid, &mut reads);
        let ctl = self.make_ctl(reads.into_iter().map(|(s, e)| (s, REdge::Any, e)).collect());
        self.emit(Instr::Wait { ctl });
        self.emit(Instr::Jmp { to: top });
        self.end_proc();
        Ok(h)
    }

    /// The wait of a timing control; returns the control of an `@*` (its triggers are set
    /// again once the body is compiled, see `set_star_ctl`).
    fn timing(&mut self, t: &Timing, body: &Stmt, sid: usize) -> R<Option<u32>> {
        match t {
            Timing::Delay(d) => {
                let ds = self.delay_ticks(d, sid)?;
                self.emit(Instr::Delay { s: ds });
            }
            Timing::Event(ec) => {
                let (ctl, followed) = self.event_ctl(ec, body, sid)?;
                // the hidden signals start from the expressions' values now: their processes
                // may not yet have seen a change this process made (`j = 1; @(a[j])`), which
                // would then look like an event
                for (h, e) in followed {
                    let w = self.sig(h).width;
                    let v = self.expr(e, sid, w, false)?;
                    self.emit(Instr::Store { t: Target::Whole { sig: h }, s: v, mode: StoreMode::Blocking });
                }
                self.emit(Instr::Wait { ctl });
                if matches!(ec, EventCtl::Star) {
                    return Ok(Some(ctl));
                }
            }
            Timing::Repeat(n, ec) => {
                let wait = Stmt::Timed { timing: Timing::Event(ec.clone()), body: Box::new(Stmt::Null), line: self.line };
                self.stmt(&Stmt::Repeat { count: n.clone(), body: Box::new(wait), line: self.line }, sid)?;
            }
        }
        Ok(None)
    }

    /// The triggers of `@*` before `body`: what it reads. Found after the body is compiled,
    /// when the scopes of its blocks (their declarations and imports) exist.
    fn set_star_ctl(&mut self, ctl: u32, body: &Stmt, sid: usize) {
        let mut reads = Vec::new();
        self.reads_stmt(body, sid, &mut reads);
        let mut trig: Vec<(u32, REdge, u32)> = reads.into_iter().map(|(s, e)| (s, REdge::Any, e)).collect();
        trig.sort_by_key(|t| (t.0, t.2, t.1 as u8));
        trig.dedup();
        self.prog.ctls[ctl as usize].triggers = trig;
    }

    pub(crate) fn stmt(&mut self, st: &Stmt, sid: usize) -> R<()> {
        match st {
            Stmt::Null => {}
            Stmt::Block { name, decls, imports, params, stmts, fork } => {
                // a fork block's automatic variables belong to each branch (see below)
                let evict = (fork.is_some() && decls.iter().any(|d| d.automatic)).then(|| self.emit(Instr::Evict { defs: Box::new([]) }));
                let bsid = if name.is_some() || !decls.is_empty() || !imports.is_empty() || !params.is_empty() {
                    let path = match name {
                        Some(n) => format!("{}.{}", self.scopes[sid].name, n),
                        None => self.scopes[sid].name.clone(),
                    };
                    // (a block compiled again, e.g. in another call of its function, has
                    // its name already)
                    if let Some(n) = name.as_ref().filter(|_| !self.code.block_scopes.contains_key(&(sid, st as *const Stmt))) {
                        self.scope_name_free(n, n, sid)?;
                    }
                    let file = self.file.clone();
                    let b = self.new_scope(path.clone(), Some(sid), file);
                    if let Some(n) = name {
                        self.scopes[sid].children.insert(n.clone(), b);
                    }
                    self.block_imports(imports, b)?;
                    self.local_params(params, b)?;
                    self.code.block_scopes.insert((sid, st as *const Stmt), b);
                    for d in decls {
                        let (mut m, mut l) = self.range_vals(&d.range, b)?;
                        let mut signed = d.signed;
                        let kind = match d.kind {
                            NetKind::Integer => {
                                (m, l) = (31, 0);
                                signed = true;
                                SigKind::Integer
                            }
                            NetKind::Int(n) => {
                                if d.range.is_none() {
                                    (m, l) = (n as i64 - 1, 0);
                                }
                                SigKind::Integer
                            }
                            NetKind::Time | NetKind::Real | NetKind::String | NetKind::Class => {
                                (m, l) = (63, 0);
                                SigKind::Reg
                            }
                            _ => SigKind::Reg,
                        };
                        let is_real = d.kind == NetKind::Real;
                        // packed dimensions: one flat vector [W-1:0], the dimensions kept
                        let mut pk: Option<Vec<(i64, i64)>> = None;
                        if !d.packed.is_empty() {
                            let mut dims = vec![(m, l)];
                            for (a, b2) in &d.packed {
                                dims.push((self.const_int(a, b)?, self.const_int(b2, b)?));
                            }
                            let w: i64 = dims.iter().map(|(x, y)| (x - y).abs() + 1).product();
                            (m, l) = (w - 1, 0);
                            pk = Some(dims);
                        }
                        let w = ((m - l).abs() + 1) as u32;
                        let layout = self.layout_for(d.ty.as_ref(), b)?;
                        // a `static` variable of a subroutine's block: one for all calls (not in
                        // a constant function call, which starts afresh)
                        let shared = d.static_ && self.subr.inline_stack.last().is_some_and(|f| !f.2 .1.ends_with("$const"));
                        // (in an automatic subroutine, block variables are automatic too)
                        let in_auto = !d.static_ && self.subr.inline_stack.last().is_some_and(|f| f.2 .1.contains('#'));
                        for dl in &d.names {
                            let key = dl as *const Declarator;
                            if shared {
                                if let Some(&s) = self.subr.static_vars.get(&key) {
                                    self.scopes[b].syms.insert(dl.name.clone(), Sym::Sig(s));
                                    continue;
                                }
                            }
                            if let Some(h) = &dl.heap {
                                if shared {
                                    return self.err(format!("'{}': a static queue or dynamic array in a subroutine's block is not supported", dl.name));
                                }
                                // a queue or dynamic array
                                let elem = self.elem_for(d.kind, w, signed, d.ty.as_deref(), b)?;
                                let s = self.new_dyn(format!("{path}.{}", dl.name), &dl.name, elem, h, b)?;
                                if d.automatic || in_auto {
                                    self.sigs.automatic.insert(s);
                                }
                                if let Some(init) = &dl.init {
                                    let ty = self.sig_type(s);
                                    let v = self.value_for(init, b, 64, ty, None)?;
                                    self.emit(Instr::Store { t: Target::Whole { sig: s }, s: v, mode: StoreMode::Blocking });
                                }
                                continue;
                            }
                            if dl.dims.len() > 1 {
                                return self.err(format!("'{}': a multi-dimensional array in a block is not supported", dl.name));
                            }
                            let (depth, lo, desc) = if let Some((a, b2)) = dl.dims.first() {
                                let (a, b2) = (self.const_int(a, b)?, self.const_int(b2, b)?);
                                (((a - b2).abs() + 1) as u32, a.min(b2), a > b2)
                            } else {
                                (0, 0, false)
                            };
                            let s =
                                self.new_signal(format!("{path}.{}", dl.name), w, signed, depth, m, l, lo, kind);
                            if (d.automatic || in_auto) && !shared {
                                self.sigs.automatic.insert(s);
                            }
                            if desc {
                                self.sigs.desc_arrays.insert(s);
                            }
                            if let Some(d) = &pk {
                                self.sigs.packed_dims.insert(s, d.clone());
                                self.scopes[b].syms.insert(format!("$flat:{}", dl.name), Sym::Sig(s));
                            }
                            if is_real {
                                self.mark_real(s);
                            }
                            if d.kind == NetKind::String {
                                self.mark_string(s);
                            }
                            if d.kind == NetKind::Event {
                                self.zero_init(s);
                            }
                            if d.kind == NetKind::Class {
                                let c = self.class_of_decl(d, b)?;
                                self.sigs.types[s as usize] = Type::Class(c);
                                self.zero_init(s);
                            }
                            self.mark_two_state(s, d.kind);
                            self.scopes[b].syms.insert(dl.name.clone(), Sym::Sig(s));
                            self.set_layout(s, &dl.name, layout.clone(), b)?;
                            if shared {
                                // its initial value once, as for a module variable
                                self.subr.static_vars.insert(key, s);
                                if let Some(init) = &dl.init {
                                    self.decl_init(init, s, kind, d.kind, is_real, depth, w, None, None, &dl.name, b)?;
                                }
                                continue;
                            }
                            if let Some(init) = &dl.init {
                                let ty = self.sig_type(s);
                                let l = self.layout(s);
                                let v = self.value_for(init, b, w, ty, l.as_deref())?;
                                self.emit(Instr::Store { t: Target::Whole { sig: s }, s: v, mode: StoreMode::Blocking });
                            }
                        }
                    }
                    b
                } else {
                    sid
                };
                // the memory of the automatic variables: private to a fork branch running here
                let mut auto_mem: Vec<(u32, u32)> = Vec::new();
                for d in decls.iter().filter(|d| d.automatic) {
                    for n in &d.names {
                        if let Some(Sym::Sig(s)) = self.scopes[bsid].syms.get(&n.name).cloned() {
                            let i = self.sig(s);
                            auto_mem.push((i.slot, rt::words(i.width) * i.depth.max(1)));
                        }
                    }
                }
                if fork.is_none() {
                    for &(lo, n) in &auto_mem {
                        self.note_frame(lo, lo + n);
                    }
                }
                let reg = match name {
                    Some(n) => {
                        let path = format!("{}.{}", self.scopes[sid].name, n);
                        self.open_block(path)
                    }
                    None => None,
                };
                // in constant evaluation: a `disable` of the block jumps to its end
                let const_block = name.is_some() && reg.is_none() && fork.is_none();
                if let (true, Some(n)) = (const_block, name) {
                    let path = format!("{}.{}", self.scopes[sid].name, n);
                    self.code.const_blocks.push((path, Vec::new()));
                }
                match fork {
                    None => {
                        for s in stmts {
                            self.stmt(s, bsid)?;
                        }
                    }
                    Some(join) => {
                        // each statement runs as its own process; the parent starts them all
                        let parent = self.code.pb.last().unwrap().proc;
                        let mut children = Vec::new();
                        for s in stmts {
                            let pname = self.scopes[bsid].name.clone();
                            let c = self.begin_proc(pname, ProcClass::Spawned);
                            self.prog.procs[c as usize].parent = parent;
                            for &ob in &self.code.open_blocks {
                                self.code.block_reg[ob].1.children.push(c);
                            }
                            self.stmt(s, bsid)?;
                            self.emit(Instr::Halt);
                            self.end_proc();
                            children.push(c);
                        }
                        let join = match join {
                            Join::All => 0,
                            Join::Any => 1,
                            Join::None => 2,
                        };
                        if let Some(e) = evict {
                            // each branch has the fork block's automatic variables of its own
                            for &c in &children {
                                let pd = &mut self.prog.procs[c as usize];
                                pd.frame.extend(auto_mem.iter().copied());
                                pd.frame = crate::codegen::merge_ranges(std::mem::take(&mut pd.frame));
                                pd.capture = true;
                            }
                            self.code.pb.last_mut().unwrap().code[e] = Instr::Evict { defs: children.clone().into_boxed_slice() };
                        }
                        if !children.is_empty() {
                            self.emit(Instr::Fork { children: children.into_boxed_slice(), join });
                        }
                    }
                }
                if const_block {
                    let (_, jumps) = self.code.const_blocks.pop().unwrap();
                    let h = self.here();
                    for j in jumps {
                        self.patch(j, h);
                    }
                }
                self.close_block(reg);
            }
            Stmt::Assign { lhs, rhs, nonblocking, timing, line } => {
                self.line = *line;
                self.check_enum_assign(lhs, rhs, sid)?;
                self.check_proc_target(lhs, sid)?;
                if *nonblocking && !self.sigs.automatic.is_empty() {
                    if let Some(n) = self.automatic_in(lhs, sid, true) {
                        return self.err(format!("nonblocking assignment to automatic variable '{n}'"));
                    }
                    if let Some(Timing::Event(EventCtl::List(l)) | Timing::Repeat(_, EventCtl::List(l))) = timing {
                        for (_, e) in l {
                            if let Some(n) = self.automatic_in(e, sid, false) {
                                return self.err(format!("automatic variable '{n}' in the event control of a nonblocking assignment"));
                            }
                        }
                    }
                }
                if self.in_program(sid) && self.lvalue_outside_program(lhs, sid) {
                    let (kind, region) = if *nonblocking { ("non-blocking", "Reactive-NBA") } else { ("blocking", "Reactive") };
                    self.warn(format!("Program {kind} assignments are not currently scheduled in the {region} region."));
                }
                if !*nonblocking && timing.is_none() && !self.code.str_ids.is_empty() && self.str_index_assign(lhs, rhs, sid)? {
                    return Ok(());
                }
                // an element of an array property (`obj.q[i] = v`)
                if self.sigs.has_dyn && !*nonblocking && timing.is_none() && matches!(lhs, Expr::Index(..)) && self.dyn_elem_assign(lhs, rhs, sid)? {
                    return Ok(());
                }
                // a property of an object
                if !self.classes.list.is_empty() && self.has_prop(lhs, sid)? {
                    if *nonblocking || timing.is_some() {
                        return self.err("a nonblocking or delayed assignment to a class property is not supported");
                    }
                    if !self.prop_assign(lhs, rhs, sid)? {
                        return self.err("an assignment to a part of a class property is not supported");
                    }
                    return Ok(());
                }
                // an element of a queue or dynamic array (also `<=`: applied at once)
                if self.sigs.has_dyn && timing.is_none() && self.dyn_elem_assign(lhs, rhs, sid)? {
                    return Ok(());
                }
                // a whole unpacked array: element by element
                if let Some(pairs) = self.array_pairs(lhs, rhs, sid)? {
                    if timing.is_some() {
                        return self.err("a timing control in an assignment of a whole array is not supported");
                    }
                    for (l, r) in pairs {
                        let st = Stmt::Assign { lhs: l, rhs: r, nonblocking: *nonblocking, timing: None, line: *line };
                        self.stmt(&st, sid)?;
                    }
                    return Ok(());
                }
                let lt = self.type_of(lhs, sid)?;
                let lw = lt.width();
                let tl = self.target_layout(lhs, sid);
                let cut = if let Type::Dyn { .. } = lt { self.bounded_pattern(rhs, lhs, sid) } else { None };
                let v = self.value_for(cut.as_ref().unwrap_or(rhs), sid, lw, lt, tl.as_ref())?;
                // the target's old array goes (the new one is a copy or a new array)
                if let (Type::Dyn { .. }, false, None) = (lt, *nonblocking, timing) {
                    self.dyn_free_target(lhs, sid);
                }
                let lv = Lhs::Expr(lhs, sid);
                match timing {
                    None => {
                        let mode = if *nonblocking { StoreMode::Nba } else { StoreMode::Blocking };
                        self.store_value(&lv, v, lw, mode)?;
                    }
                    Some(Timing::Delay(d)) if *nonblocking => {
                        let ds = self.delay_ticks(d, sid)?;
                        self.store_value(&lv, v, lw, StoreMode::NbaDelay(ds))?;
                    }
                    Some(t) if *nonblocking => {
                        // `x <= @(e) v`: the value (and the target's indices) now; a thread of
                        // its own waits for the event and makes the update while this process
                        // goes on (each pending update an instance with its own copies)
                        let tv = self.alloc(lw);
                        let hi = self.prog.mem.len() as u32;
                        let mut frame = vec![(tv, hi - tv)];
                        let parent = self.code.pb.last().unwrap().proc;
                        let evict = self.emit(Instr::Evict { defs: Box::new([]) });
                        self.emit(Instr::Copy { d: tv, s: v, w: lw });
                        let lhs2 = self.capture_indices(lhs, sid, &mut frame)?;
                        let c = self.begin_proc(self.scopes[sid].name.clone(), ProcClass::Spawned);
                        self.prog.procs[c as usize].parent = parent;
                        self.timing(t, &Stmt::Null, sid)?;
                        self.store_value(&Lhs::Expr(&lhs2, sid), tv, lw, StoreMode::Nba)?;
                        self.emit(Instr::Halt);
                        self.end_proc();
                        let pd = &mut self.prog.procs[c as usize];
                        pd.frame.extend(frame);
                        pd.frame = crate::codegen::merge_ranges(std::mem::take(&mut pd.frame));
                        pd.capture = true;
                        self.code.pb.last_mut().unwrap().code[evict] = Instr::Evict { defs: Box::new([c]) };
                        self.emit(Instr::Fork { children: Box::new([c]), join: 3 });
                    }
                    Some(t) => {
                        // intra-assignment timing: evaluate, wait, assign
                        let tv = self.ptmp(lw);
                        self.emit(Instr::Copy { d: tv, s: v, w: lw });
                        self.timing(t, &Stmt::Null, sid)?;
                        let mode = if *nonblocking { StoreMode::Nba } else { StoreMode::Blocking };
                        self.store_value(&lv, tv, lw, mode)?;
                    }
                }
            }
            Stmt::If { cond, then_s, else_s, line } => {
                self.line = *line;
                let (c, cw) = self.truth(cond, sid)?;
                let jz = self.emit(Instr::Jz { c, w: cw, to: 0 });
                self.stmt(then_s, sid)?;
                if let Some(e) = else_s {
                    let j = self.emit(Instr::Jmp { to: 0 });
                    let h = self.here();
                    self.patch(jz, h);
                    self.stmt(e, sid)?;
                    let h = self.here();
                    self.patch(j, h);
                } else {
                    let h = self.here();
                    self.patch(jz, h);
                }
            }
            Stmt::Case { kind, expr, arms, default, line } => {
                self.line = *line;
                let et = self.type_of(expr, sid)?;
                let mut real = et.is_real();
                let (mut w, mut s) = et.ws();
                for (labels, _) in arms {
                    for l in labels {
                        let lt = self.type_of(l, sid)?;
                        real |= lt.is_real();
                        let (lw, ls) = lt.ws();
                        w = w.max(lw);
                        s &= ls;
                    }
                }
                // IEEE 1364-2005 9.5: with a real expression or item all are compared as reals
                // (each converted with its own signedness)
                if real {
                    w = 64;
                }
                let ev = if real { self.rexpr(expr, sid)? } else { self.expr(expr, sid, w, s)? };
                let e = self.ptmp(w);
                self.emit(Instr::Copy { d: e, s: ev, w });
                let op = match kind {
                    CaseKind::Case => CmpOp::CaseEq,
                    CaseKind::CaseZ => CmpOp::ZEq,
                    CaseKind::CaseX => CmpOp::XEq,
                };
                let mut to_end = Vec::new();
                for (labels, body) in arms {
                    let mut to_body = Vec::new();
                    let mut next = None;
                    for (k, l) in labels.iter().enumerate() {
                        if let Some(n) = next.take() {
                            let h = self.here();
                            self.patch(n, h);
                        }
                        let t = self.tmp(1);
                        if real {
                            let lv = self.rexpr(l, sid)?;
                            self.emit(Instr::RCmp { op: CmpOp::Eq, d: t, a: e, b: lv });
                        } else {
                            let lv = self.expr(l, sid, w, s)?;
                            self.emit(Instr::Cmp { op, d: t, a: e, b: lv, w });
                        }
                        let jz = self.emit(Instr::Jz { c: t, w: 1, to: 0 });
                        if k + 1 < labels.len() {
                            to_body.push(self.emit(Instr::Jmp { to: 0 }));
                        }
                        next = Some(jz);
                    }
                    let h = self.here();
                    for j in to_body {
                        self.patch(j, h);
                    }
                    self.stmt(body, sid)?;
                    to_end.push(self.emit(Instr::Jmp { to: 0 }));
                    if let Some(n) = next {
                        let h = self.here();
                        self.patch(n, h);
                    }
                }
                if let Some(d) = default {
                    self.stmt(d, sid)?;
                }
                let h = self.here();
                for j in to_end {
                    self.patch(j, h);
                }
            }
            Stmt::For { init, cond, step, body, line } => {
                self.line = *line;
                self.stmt(init, sid)?;
                let top = self.here();
                let (c, cw) = self.truth(cond, sid)?;
                let jz = self.emit(Instr::Jz { c, w: cw, to: 0 });
                self.code.pb.last_mut().unwrap().loops.push((Vec::new(), Vec::new()));
                self.stmt(body, sid)?;
                let (brk, cont) = self.code.pb.last_mut().unwrap().loops.pop().unwrap();
                let h = self.here();
                for j in cont {
                    self.patch(j, h);
                }
                self.stmt(step, sid)?;
                self.emit(Instr::Jmp { to: top });
                let h = self.here();
                self.patch(jz, h);
                for j in brk {
                    self.patch(j, h);
                }
            }
            Stmt::While { cond, body, line } => {
                self.line = *line;
                let top = self.here();
                let (c, cw) = self.truth(cond, sid)?;
                let jz = self.emit(Instr::Jz { c, w: cw, to: 0 });
                self.loop_body(body, sid, top)?;
                self.emit(Instr::Jmp { to: top });
                let h = self.here();
                self.patch(jz, h);
                self.finish_loop(h);
            }
            Stmt::DoWhile { cond, body, line } => {
                self.line = *line;
                let top = self.here();
                self.code.pb.last_mut().unwrap().loops.push((Vec::new(), Vec::new()));
                self.stmt(body, sid)?;
                let ctop = self.here();
                let (c, cw) = self.truth(cond, sid)?;
                let jz = self.emit(Instr::Jz { c, w: cw, to: 0 });
                self.emit(Instr::Jmp { to: top });
                let h = self.here();
                self.patch(jz, h);
                let (brk, cont) = self.code.pb.last_mut().unwrap().loops.pop().unwrap();
                for j in cont {
                    self.patch(j, ctop);
                }
                for j in brk {
                    self.patch(j, h);
                }
            }
            Stmt::Repeat { count, body, line } => {
                self.line = *line;
                let (cv, cw, cs) = self.self_expr(count, sid)?;
                let cnt = self.ptmp(cw);
                self.emit(Instr::Copy { d: cnt, s: cv, w: cw });
                let zero = self.const_slot(&Const::zero(cw));
                let one = self.const_slot(&Const::from_i64(1, cw, false));
                let top = self.here();
                let t = self.tmp(1);
                self.emit(Instr::Cmp { op: if cs { CmpOp::GtS } else { CmpOp::Gt }, d: t, a: cnt, b: zero, w: cw });
                let jz = self.emit(Instr::Jz { c: t, w: 1, to: 0 });
                self.emit(Instr::Bin { op: rt::BinOp::Sub, d: cnt, a: cnt, b: one, w: cw });
                self.loop_body(body, sid, top)?;
                self.emit(Instr::Jmp { to: top });
                let h = self.here();
                self.patch(jz, h);
                self.finish_loop(h);
            }
            Stmt::Forever { body, line } => {
                self.line = *line;
                let top = self.here();
                self.loop_body(body, sid, top)?;
                self.emit(Instr::Jmp { to: top });
                let h = self.here();
                self.finish_loop(h);
            }
            Stmt::Timed { timing, body, line } => {
                self.line = *line;
                let star = self.timing(timing, body, sid)?;
                self.stmt(body, sid)?;
                if let Some(ctl) = star {
                    self.set_star_ctl(ctl, body, sid);
                }
            }
            Stmt::Wait { cond, body, line } => {
                self.line = *line;
                let top = self.here();
                let (c, cw) = self.truth(cond, sid)?;
                let jz = self.emit(Instr::Jz { c, w: cw, to: 0 });
                let jdone = self.emit(Instr::Jmp { to: 0 });
                let h = self.here();
                self.patch(jz, h);
                let mut reads = Vec::new();
                self.reads_expr(cond, sid, &mut reads);
                let ctl = self.make_ctl(reads.into_iter().map(|(s, e)| (s, REdge::Any, e)).collect());
                self.emit(Instr::Wait { ctl });
                self.emit(Instr::Jmp { to: top });
                let h = self.here();
                self.patch(jdone, h);
                self.stmt(body, sid)?;
            }
            Stmt::SysTask { name, args, line } => {
                self.line = *line;
                self.sys_task(name, args, sid)?;
            }
            Stmt::MethodCall { obj, name, args, line } => {
                self.line = *line;
                if self.sigs.has_dyn && self.dyn_method_task(obj, name, args, sid)? {
                    return Ok(());
                }
                self.method_call(obj, name, args, sid)?;
            }
            Stmt::StaticCall { class, name, args, line } => {
                self.line = *line;
                self.static_call(class, name, args, sid)?;
            }
            Stmt::TaskCall { name, args, line } => {
                self.line = *line;
                if self.sigs.has_dyn && self.dyn_task(name, args, sid)? {
                    return Ok(());
                }
                if self.code.str_ids.is_empty() || !self.str_task(name, args, sid)? {
                    self.call(name, args, sid, true)?;
                }
            }
            Stmt::Force { lhs, rhs, line } => {
                // force / procedural assign: a spawned process keeps the target at the value of
                // the expression (re-applied whenever the expression or the target changes)
                // until release / deassign stops it
                self.line = *line;
                let auto = match self.automatic_in(lhs, sid, true) {
                    Some(n) => Some(n),
                    None => rhs.as_ref().and_then(|r| self.automatic_in(r, sid, false)),
                };
                if let Some(n) = auto {
                    return self.err(format!("automatic variable '{n}' in a procedural continuous assignment (assign, deassign, force, release)"));
                }
                self.check_const_selects(lhs, sid)?;
                let mut bases = Vec::new();
                self.lvalue_bases(lhs, sid, &mut bases);
                let key = format!("$force:{:?}:{:?}", bases, lhs);
                for &b in &bases {
                    self.prog.signals[b as usize].forced = true;
                }
                let stop = self.disable_target(vec![key.clone()]);
                self.emit(Instr::Disable { target: stop });
                if rhs.is_none() {
                    // the bits are no longer forced; a net takes its drivers' value again
                    let mut ts = Vec::new();
                    self.targets(lhs, sid, &mut ts)?;
                    for (t, _) in ts.into_iter() {
                        if let Some(t) = t {
                            self.emit(Instr::Unforce { t });
                        }
                    }
                    for &b in &bases {
                        if self.sig(b).kind == SigKind::Wire {
                            self.emit(Instr::Kick { sig: b });
                        }
                    }
                }
                if let Some(rhs) = rhs {
                    // the forced value applies at once ...
                    let lt = self.type_of(lhs, sid)?;
                    let lw = lt.width();
                    let tl = self.target_layout(lhs, sid);
                    let v = self.value_for(rhs, sid, lw, lt, tl.as_ref())?;
                    self.store_value(&Lhs::Expr(lhs, sid), v, lw, StoreMode::Force)?;
                    let parent = self.code.pb.last().unwrap().proc;
                    let name = self.scopes[sid].name.clone();
                    let c = self.begin_proc(name, ProcClass::Spawned);
                    self.prog.procs[c as usize].parent = parent;
                    let reg = self.open_block(key);
                    if let Some(r) = reg {
                        self.code.block_reg[r].1.children.push(c);
                    }
                    let top = self.here();
                    // ... and is kept up to date by the spawned process
                    let tl = self.target_layout(lhs, sid);
                    let v = self.value_for(rhs, sid, lw, lt, tl.as_ref())?;
                    self.store_value(&Lhs::Expr(lhs, sid), v, lw, StoreMode::Reforce)?;
                    let mut reads = Vec::new();
                    self.reads_expr(rhs, sid, &mut reads);
                    self.reads_lvalue(lhs, sid, &mut reads);
                    reads.extend(bases.iter().map(|&b| (b, NONE)));
                    let ctl = self.make_ctl(reads.into_iter().map(|(s, e)| (s, REdge::Any, e)).collect());
                    self.emit(Instr::Wait { ctl });
                    self.emit(Instr::Jmp { to: top });
                    self.close_block(reg);
                    self.end_proc();
                    self.emit(Instr::Fork { children: vec![c].into_boxed_slice(), join: 2 });
                }
            }
            Stmt::Disable { name, line } => {
                self.line = *line;
                if name == "fork" {
                    // SystemVerilog `disable fork`: the branches this process started
                    self.emit(Instr::Disable { target: NONE });
                } else {
                    // resolved after code generation: the block may be in another process
                    let mut cands = Vec::new();
                    let mut sc = Some(sid);
                    while let Some(x) = sc {
                        cands.push(format!("{}.{}", self.scopes[x].name, name));
                        sc = self.scopes[x].parent;
                    }
                    // in constant evaluation: an enclosing named block of the evaluated code
                    if let Some(k) = self.code.const_blocks.iter().rposition(|(p, _)| cands.contains(p)) {
                        let j = self.emit(Instr::Jmp { to: 0 });
                        self.code.const_blocks[k].1.push(j);
                        return Ok(());
                    }
                    cands.extend(self.hier.tops.keys().map(|t| format!("{t}.{}", name.split_once('.').map(|x| x.1).unwrap_or(""))));
                    let target = self.disable_target(cands);
                    self.emit(Instr::Disable { target });
                }
            }
            Stmt::Trigger { name, line } => {
                self.line = *line;
                let e = if name.contains('.') {
                    Expr::Hier(name.split('.').map(String::from).collect())
                } else {
                    Expr::Ident(name.clone())
                };
                let Some(expr::Base::Sig(s)) = self.resolve_base(&e, sid)? else {
                    return self.err(format!("'{name}' is not an event"));
                };
                // the trigger time, for `.triggered` (before the waiters wake)
                let mut te = e.clone();
                match &mut te {
                    Expr::Ident(n) => *n = format!("{n}$trig"),
                    Expr::Hier(p) => {
                        let l = p.last_mut().unwrap();
                        *l = format!("{l}$trig");
                    }
                    _ => {}
                }
                // (only module-level events have one)
                let hidden = match &te {
                    Expr::Ident(n) => self.lookup(n, sid),
                    Expr::Hier(p) => self.lookup_hier(p, sid),
                    _ => None,
                };
                if let Some(Sym::Sig(ht)) = hidden {
                    let d = self.tmp(64);
                    self.emit(Instr::SysF { f: rt::SysFn::Time(0), d });
                    self.emit(Instr::Store { t: Target::Whole { sig: ht }, s: d, mode: StoreMode::Blocking });
                }
                // a trigger toggles the event variable: every waiter sees a change
                let slot = self.sig(s).slot;
                let t = self.tmp(1);
                self.emit(Instr::Un { op: rt::UnOp::Not, d: t, s: slot, w: 1 });
                self.emit(Instr::Store { t: Target::Whole { sig: s }, s: t, mode: StoreMode::Blocking });
            }
            Stmt::Return { value, line } => {
                self.line = *line;
                let Some((ret, _)) = self.code.pb.last().unwrap().rets.last().cloned() else {
                    return self.err("return outside of a function or task");
                };
                if value.is_some() && ret.is_none() {
                    return self.err("a task or void function cannot return a value");
                }
                if let (Some(v), Some(r)) = (value, ret) {
                    if let Some(&id) = self.enums.sigs.get(&r) {
                        self.check_enum(id, v, sid)?;
                    }
                    let w = self.sig(r).width;
                    let real = self.sig_type(r);
                    let l = self.layout(r);
                    let val = self.value_for(v, sid, w, real, l.as_deref())?;
                    self.emit(Instr::Store { t: Target::Whole { sig: r }, s: val, mode: StoreMode::Blocking });
                }
                let j = self.emit(Instr::Jmp { to: 0 });
                self.code.pb.last_mut().unwrap().rets.last_mut().unwrap().1.push(j);
            }
            Stmt::Discard { e, line } => {
                self.line = *line;
                self.self_expr(e, sid)?;
            }
            Stmt::WaitFork { line } => {
                self.line = *line;
                self.emit(Instr::WaitFork);
            }
            Stmt::Break { line } | Stmt::Continue { line } => {
                self.line = *line;
                let is_break = matches!(st, Stmt::Break { .. });
                let j = self.emit(Instr::Jmp { to: 0 });
                match self.code.pb.last_mut().unwrap().loops.last_mut() {
                    Some(l) => {
                        if is_break {
                            l.0.push(j)
                        } else {
                            l.1.push(j)
                        }
                    }
                    None => return self.err("break/continue outside of a loop"),
                }
            }
        }
        Ok(())
    }

    fn loop_body(&mut self, body: &Stmt, sid: usize, cont_target: u32) -> R<()> {
        self.code.pb.last_mut().unwrap().loops.push((Vec::new(), Vec::new()));
        self.stmt(body, sid)?;
        let cont = std::mem::take(&mut self.code.pb.last_mut().unwrap().loops.last_mut().unwrap().1);
        for j in cont {
            self.patch(j, cont_target);
        }
        Ok(())
    }

    fn finish_loop(&mut self, end: u32) {
        let (brk, _) = self.code.pb.last_mut().unwrap().loops.pop().unwrap();
        for j in brk {
            self.patch(j, end);
        }
    }

    pub(crate) fn fmt_args(&mut self, args: &[Option<Expr>], sid: usize) -> R<Vec<FmtArg>> {
        let mut v = Vec::new();
        for a in args {
            v.push(match a {
                None => FmtArg::Empty,
                Some(Expr::Str(s)) => FmtArg::Lit(s.clone()),
                // a parameter with a string literal value prints as text (as the reference)
                Some(e @ Expr::Ident(n)) if self.is_str_param(n, sid) => {
                    let (s, w, _) = self.self_expr(e, sid)?;
                    FmtArg::Str { s, w }
                }
                Some(e) if self.type_of(e, sid)?.is_str() => FmtArg::SVar { s: self.str_expr(e, sid)? },
                Some(e) if self.type_of(e, sid)?.is_real() => {
                    let fixed = matches!(e, Expr::SysCall(n, a) if n == "$realtime" && a.is_empty())
                        .then(|| (self.scopes[sid].tunit - self.scopes[sid].tprec).clamp(0, 18) as u8);
                    FmtArg::Real { s: self.rexpr(e, sid)?, fixed }
                }
                // a string value (an enum's .name())
                Some(Expr::SysCall(n, a)) if n == "$__str" => {
                    let (s, w, _) = self.self_expr(&a[0], sid)?;
                    FmtArg::Str { s, w }
                }
                Some(e) => {
                    let (s, w, signed) = self.self_expr(e, sid)?;
                    FmtArg::Val { s, w, signed }
                }
            });
        }
        Ok(v)
    }

    /// The target of a procedural assignment is a variable (IEEE 1800-2017 10.4, table
    /// 10-1), never a net; it is noted for the check against continuous assignments.
    fn check_proc_target(&mut self, lhs: &Expr, sid: usize) -> R<()> {
        if self.code.pb.last().is_none_or(|p| p.proc == NONE) {
            return Ok(()); // a constant function call
        }
        let mut b = lhs;
        while let Expr::Member(a, _) | Expr::Index(a, _) | Expr::Range(a, ..) | Expr::IndexedRange(a, ..) = b {
            b = a;
        }
        let sym = match b {
            Expr::Concat(v) => return v.iter().try_for_each(|x| self.check_proc_target(x, sid)),
            Expr::Ident(n) => self.lookup(n, sid),
            Expr::Hier(p) => self.lookup_hier(p, sid),
            _ => None,
        };
        if let Some(Sym::Sig(s)) = sym {
            if self.sig(s).kind == SigKind::Wire {
                let n = self.sig(s).name.rsplit('.').next().unwrap_or_default().to_string();
                return self.err(format!("procedural assignment to net '{n}' (only variables take procedural assignments)"));
            }
            let c = self.canon(s);
            self.code.proc_writes.insert(c);
        }
        Ok(())
    }

    /// The name of an automatic variable `e` writes (`lvalue`) or reads, if any.
    fn automatic_in(&self, e: &Expr, sid: usize, lvalue: bool) -> Option<String> {
        if self.sigs.automatic.is_empty() {
            return None;
        }
        let mut member = false;
        let mut v = Vec::new();
        if lvalue {
            // (through members and selects: `s.x[1] <= v` writes `s`)
            let mut b = e;
            while let Expr::Member(a, _) | Expr::Index(a, _) | Expr::Range(a, ..) | Expr::IndexedRange(a, ..) = b {
                member |= matches!(b, Expr::Member(..));
                b = a;
            }
            if let Expr::Concat(_) = b {
                self.lvalue_bases(b, sid, &mut v);
            } else {
                match b {
                    Expr::Ident(n) => {
                        if let Some(Sym::Sig(s)) = self.lookup(n, sid) {
                            v.push(s);
                        }
                    }
                    Expr::Hier(p) => {
                        if let Some(Sym::Sig(s)) = self.lookup_hier(p, sid) {
                            v.push(s);
                        }
                    }
                    _ => {}
                }
            }
        } else {
            let mut r = Vec::new();
            self.reads_expr(e, sid, &mut r);
            v = r.into_iter().map(|x| x.0).collect();
        }
        // (a property through an automatic handle is in the object, not in the variable)
        let s = v.into_iter().find(|&s| self.sigs.automatic.contains(&s) && !(member && matches!(self.sig_type(s), Type::Class(_))))?;
        Some(self.sig(s).name.rsplit('.').next().unwrap_or_default().to_string())
    }

    fn sys_task(&mut self, name: &str, args: &[Option<Expr>], sid: usize) -> R<()> {
        let scope = self.scopes[sid].name.clone();
        // (these read their arguments later, when the variables may be gone)
        if matches!(name, "$monitor" | "$strobe" | "$fstrobe" | "$fmonitor") || name.strip_suffix(['b', 'h', 'o']).is_some_and(|b| matches!(b, "$monitor" | "$strobe" | "$fstrobe" | "$fmonitor")) {
            for a in args.iter().flatten() {
                if let Some(n) = self.automatic_in(a, sid, false) {
                    return self.err(format!("automatic variable '{n}' as an argument of {name}"));
                }
            }
        }
        let sys = |t: SysTask| Instr::Sys(Box::new(t));
        // $fdisplayb, $strobeh, $monitoro, ...: the default radix of unformatted arguments
        let (name, radix) = match name.strip_suffix(['b', 'h', 'o']) {
            Some(base)
                if matches!(
                    base,
                    "$display" | "$write" | "$fdisplay" | "$fwrite" | "$strobe" | "$fstrobe" | "$monitor" | "$fmonitor" | "$swrite"
                ) =>
            {
                (base, name.chars().last().unwrap())
            }
            _ => (name, 'd'),
        };
        match name {
            "$display" | "$write" => {
                let args = self.fmt_args(args, sid)?;
                let nl = name.starts_with("$display");
                let texp = self.texp(sid);
                self.emit(sys(SysTask::Display { nl, args, scope, fd: None, prefix: "", texp, radix }));
            }
            "$error" | "$warning" | "$info" | "$fatal" => {
                let mut a = args;
                if name == "$fatal" && !a.is_empty() && !matches!(a[0], Some(Expr::Str(_))) {
                    a = &a[1..];
                }
                let prefix = match name {
                    "$error" => "ERROR: ",
                    "$warning" => "WARNING: ",
                    "$info" => "INFO: ",
                    _ => "FATAL: ",
                };
                // `ERROR: file:line: message` and the time and scope below it
                let mut v = vec![Some(Expr::Str(format!("{prefix}{}: ", self.loc()).replace('%', "%%")))];
                v.extend(a.iter().cloned());
                let pad = " ".repeat(prefix.len());
                v.push(Some(Expr::Str(format!("\n{pad}Time: %0d  Scope: %m"))));
                v.push(Some(Expr::SysCall("$__ticks".into(), Vec::new())));
                let args = self.fmt_args(&v, sid)?;
                let texp = self.texp(sid);
                self.emit(sys(SysTask::Display { nl: true, args, scope, fd: None, prefix: "", texp, radix: 'd' }));
                if name == "$fatal" {
                    self.emit(sys(SysTask::Finish { loc: String::new(), stop: false, fatal: true }));
                }
            }
            "$__unhandled_case" | "$__unhandled_if" => {
                let what = if name == "$__unhandled_if" {
                    "no condition is true in a priority or unique if statement"
                } else {
                    "value is unhandled for priority or unique case statement"
                };
                let msg = format!("WARNING: {}: {what}\n         Time: %0d  Scope: %m", self.loc());
                let a = [Some(Expr::Str(msg)), Some(Expr::SysCall("$__ticks".into(), Vec::new()))];
                let args = self.fmt_args(&a, sid)?;
                let texp = self.texp(sid);
                self.emit(sys(SysTask::Display { nl: true, args, scope, fd: None, prefix: "", texp, radix: 'd' }));
            }
            "$fdisplay" | "$fwrite" => {
                let Some(Some(fd)) = args.first() else { return self.err(format!("{name} needs a file descriptor")) };
                let (fs, fw, _) = self.self_expr(fd, sid)?;
                let args = self.fmt_args(&args[1..], sid)?;
                let texp = self.texp(sid);
                self.emit(sys(SysTask::Display { nl: name == "$fdisplay", args, scope, fd: Some((fs, fw)), prefix: "", texp, radix }));
            }
            "$fclose" => {
                if let Some(Some(fd)) = args.first() {
                    let (fs, _, _) = self.self_expr(fd, sid)?;
                    let loc = format!("{}", self.loc());
                    self.emit(sys(SysTask::FClose { s: fs, loc }));
                }
            }
            "$monitor" | "$strobe" | "$fstrobe" => {
                let is_mon = name == "$monitor";
                let class = ProcClass::Postponed;
                let p = self.begin_proc(scope.clone(), class);
                let top = self.here();
                // `$fstrobe(fd, ...)`: the descriptor too is read at the end of the time step
                let (fd, args) = if name == "$fstrobe" {
                    let Some((Some(fd), rest)) = args.split_first() else { return self.err("$fstrobe needs a file descriptor") };
                    let (fs, fw, _) = self.self_expr(fd, sid)?;
                    (Some((fs, fw)), rest)
                } else {
                    (None, &args[..])
                };
                let fargs = self.fmt_args(args, sid)?;
                if is_mon {
                    let texp = self.texp(sid);
                    self.emit(sys(SysTask::MonitorPrint { proc: p, args: fargs, scope: scope.clone(), texp, radix }));
                    // IEEE 1364-2005 17.1.3: a change of an argument's value prints (not of
                    // $time, $stime, $realtime): a select or an expression is followed by a
                    // hidden signal, so a change of other bits of its signals does not print
                    let mut trig = Vec::new();
                    for e in args.iter().flatten() {
                        match e {
                            Expr::Str(_) => {}
                            Expr::SysCall(n, v) if v.is_empty() && matches!(n.as_str(), "$time" | "$stime" | "$realtime") => {}
                            Expr::Ident(_) | Expr::Hier(_) => {
                                let mut reads = Vec::new();
                                self.reads_expr(e, sid, &mut reads);
                                trig.extend(reads.into_iter().map(|(s, x)| (s, REdge::Any, x)));
                            }
                            _ => {
                                let t = self.type_of(e, sid)?;
                                // (calls must not run in another process: as before, the reads)
                                if !matches!(t, types::Type::Vec { .. }) || crate::expr::has_call(e) {
                                    let mut reads = Vec::new();
                                    self.reads_expr(e, sid, &mut reads);
                                    trig.extend(reads.into_iter().map(|(s, x)| (s, REdge::Any, x)));
                                } else {
                                    let h = self.event_signal(e, sid)?;
                                    trig.push((h, REdge::Any, NONE));
                                }
                            }
                        }
                    }
                    let ctl = self.make_ctl(trig);
                    self.emit(Instr::Wait { ctl });
                    self.emit(Instr::Jmp { to: top });
                } else {
                    let texp = self.texp(sid);
                    self.emit(sys(SysTask::Display { nl: true, args: fargs, scope: scope.clone(), fd, prefix: "", texp, radix }));
                    self.emit(Instr::Halt);
                }
                self.end_proc();
                if is_mon {
                    self.emit(sys(SysTask::Monitor { proc: p }));
                } else {
                    self.emit(sys(SysTask::Strobe { proc: p }));
                }
            }
            "$sformat" | "$swrite" => {
                let Some(Some(var)) = args.first() else {
                    return self.err(format!("{name} needs a variable"));
                };
                let ty = self.type_of(var, sid)?;
                let vw = ty.width();
                let fargs = self.fmt_args(&args[1..], sid)?;
                let d = self.tmp(vw);
                let texp = self.texp(sid);
                let radix = if name == "$swrite" { radix } else { 'd' };
                // (w 0: the handle of a string, for a string variable)
                let w = if ty.is_str() { 0 } else { vw };
                let nul_warn = format!("{}: {name}", self.loc());
                self.emit(sys(SysTask::SFormat { d, w, args: fargs, scope, texp, radix, nul_warn }));
                self.store_value(&Lhs::Expr(var, sid), d, vw, StoreMode::Blocking)?;
            }
            "$timeformat" => {
                let f = if args.is_empty() {
                    None
                } else {
                    let int = |c: &mut Self, i: usize, d: i64| -> R<i64> {
                        match args.get(i) {
                            Some(Some(e)) => c.const_int(e, sid),
                            _ => Ok(d),
                        }
                    };
                    let units = int(self, 0, self.gprec as i64)? as i32;
                    let prec = int(self, 1, 0)?.max(0) as usize;
                    let suffix = match args.get(2) {
                        Some(Some(Expr::Str(s))) => s.clone(),
                        Some(Some(_)) => return self.err("$timeformat: the suffix must be a string literal"),
                        _ => String::new(),
                    };
                    let width = int(self, 3, 20)?.max(0) as usize;
                    Some(rt::fmt::TimeFormat { units, prec, suffix, width })
                };
                self.emit(sys(SysTask::TimeFormat(f)));
            }
            "$printtimescale" => {
                // resolved now: the unit and precision of the module the named object is in;
                // the name is printed as given (bit selects of vectors as [i:i])
                let (target, item) = match args.first() {
                    Some(Some(e)) => {
                        let (base, sel) = match e {
                            Expr::Index(b, i) => (&**b, Some(&**i)),
                            _ => (e, None),
                        };
                        let parts: Vec<String> = match base {
                            Expr::Ident(n) => vec![n.clone()],
                            Expr::Hier(p) => p.clone(),
                            _ => return self.err("$printtimescale needs a scope name"),
                        };
                        // the longest prefix that names a scope
                        let mut t = None;
                        for k in (0..=parts.len()).rev() {
                            if let Some(x) = self.hier_scope(&parts[..k], sid) {
                                t = Some((x, k));
                                break;
                            }
                        }
                        let Some((t, k)) = t else {
                            return self.err(format!("$printtimescale: unknown scope '{}'", parts.join(".")));
                        };
                        let mut name = self.scopes[t].name.clone();
                        for p in &parts[k..] {
                            name = format!("{name}.{p}");
                        }
                        if let Some(i) = sel {
                            let iv = self.const_value(i, sid).unwrap_or(0);
                            let is_array = matches!(self.resolve_base(base, sid), Ok(Some(expr::Base::Sig(s))) if self.sig(s).depth > 0);
                            name = if is_array { format!("{name}[{iv}]") } else { format!("{name}[{iv}:{iv}]") };
                        }
                        (t, Some(name))
                    }
                    _ => {
                        // the enclosing module instance
                        (self.instance_scope(sid), None)
                    }
                };
                let unit = |e: i32| -> String {
                    let base = ["s", "ms", "us", "ns", "ps", "fs"][(-e.div_euclid(3)).clamp(0, 5) as usize];
                    format!("{}{base}", ["1", "10", "100"][e.rem_euclid(3) as usize])
                };
                let sc = &self.scopes[target];
                let pkg = self.is_pkg_scope(target);
                let name = item.unwrap_or_else(|| if pkg { format!("{}::", sc.name) } else { sc.name.clone() });
                let msg = format!("Time scale of ({name}) is {} / {}", unit(sc.tunit), unit(sc.tprec));
                self.emit(sys(SysTask::Display {
                    nl: true,
                    args: vec![rt::FmtArg::Lit(msg.replace('%', "%%"))],
                    scope,
                    fd: None,
                    prefix: "",
                    texp: 0,
                    radix: 'd',
                }));
            }
            "$monitoron" => {
                self.emit(sys(SysTask::MonitorOn(true)));
            }
            "$monitoroff" => {
                self.emit(sys(SysTask::MonitorOn(false)));
            }
            "$finish" | "$stop" => {
                // $finish(0) prints nothing; otherwise "file:line: $finish called at <time> (<unit>)"
                let quiet = matches!(args.first(), Some(Some(Expr::Num(n))) if n.digits == "0");
                let loc = if quiet { String::new() } else { format!("{}", self.loc()) };
                self.emit(sys(SysTask::Finish { loc, stop: name == "$stop", fatal: false }));
            }
            "$dumpfile" => {
                if let Some(Some(Expr::Str(f))) = args.first() {
                    self.emit(sys(SysTask::DumpFile(f.clone())));
                }
            }
            "$dumpvars" => {
                self.emit(sys(SysTask::DumpVars));
            }
            "$dumpon" => {
                self.emit(sys(SysTask::DumpOn(true)));
            }
            "$dumpoff" => {
                self.emit(sys(SysTask::DumpOn(false)));
            }
            "$dumpall" => {
                self.emit(sys(SysTask::DumpAll));
            }
            "$dumpflush" => {
                self.emit(sys(SysTask::DumpFlush));
            }
            "$dumplimit" => {}
            "$readmemh" | "$readmemb" | "$writememh" | "$writememb" => {
                let file = match args.first() {
                    Some(Some(e)) => self.file_name(e, sid)?,
                    _ => return self.err(format!("{name}: missing file name")),
                };
                let sig = match args.get(1) {
                    Some(Some(e)) => match self.array_of(e, sid) {
                        Some(s) => s,
                        None => return self.err(format!("{name}: second argument must be an array")),
                    },
                    _ => return self.err(format!("{name}: missing array argument")),
                };
                let opt = |i: usize, c: &mut Self| -> R<Option<(u32, u32)>> {
                    match args.get(i) {
                        Some(Some(e)) => {
                            let (s, w, _) = c.self_expr(e, sid)?;
                            Ok(Some((s, w)))
                        }
                        _ => Ok(None),
                    }
                };
                let start = opt(2, self)?;
                let end = opt(3, self)?;
                let loc = format!("{}", self.loc());
                let hex = name.ends_with('h');
                if name.starts_with("$write") {
                    self.emit(sys(SysTask::WriteMem { hex, file, sig, start, end }));
                } else {
                    if start.is_none() && self.sigs.desc_arrays.contains(&sig) {
                        // the 1364-2005 change of the default start address (as the reference
                        // reports it)
                        let msg = format!(
                            "WARNING: {loc}: {name}: The behaviour for reg[...] mem[N:0]; {name}(\"...\", mem); changed in the 1364-2005 standard. To avoid ambiguity, use mem[0:N] or explicit range parameters {name}(\"...\", mem, start, stop);. Defaulting to 1364-2005 behavior."
                        );
                        let a = [Some(Expr::Str(msg.replace('%', "%%")))];
                        let args = self.fmt_args(&a, sid)?;
                        let texp = self.texp(sid);
                        self.emit(sys(SysTask::Display { nl: true, args, scope: scope.clone(), fd: None, prefix: "", texp, radix: 'd' }));
                    }
                    self.emit(sys(SysTask::ReadMem { hex, file, sig, start, end, loc }));
                }
            }
            "$fgetc" | "$ungetc" | "$fgets" | "$feof" | "$ftell" | "$fseek" | "$rewind" | "$fscanf" | "$sscanf" => {
                // called as a task: the result is dropped
                let a: Vec<Expr> = args.iter().map(|a| a.clone().unwrap_or(Expr::Str(String::new()))).collect();
                self.file_read(name, &a, sid)?;
            }
            "$fflush" => {
                // (arguments that are calls still run)
                for e in args.iter().flatten() {
                    if crate::expr::has_call(e) {
                        self.self_expr(e, sid)?;
                    }
                }
                self.emit(sys(SysTask::Flush));
            }
            _ => self.warn(format!("system task '{name}' is not supported, ignored")),
        }
        Ok(())
    }

    // ------------------------------------------------------------------ read sets
    pub(crate) fn reads_stmt(&self, st: &Stmt, sid: usize, out: &mut Vec<(u32, u32)>) {
        let mut locals: Vec<u32> = Vec::new();
        self.reads_stmt_inner(st, sid, out, &mut locals);
        out.retain(|s| !locals.contains(&s.0));
        out.sort_unstable();
        out.dedup();
    }

    fn reads_stmt_inner(&self, st: &Stmt, sid: usize, out: &mut Vec<(u32, u32)>, locals: &mut Vec<u32>) {
        match st {
            Stmt::Null | Stmt::Disable { .. } | Stmt::Trigger { .. } | Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::WaitFork { .. } => {}
            Stmt::Discard { e, .. } => self.reads_expr(e, sid, out),
            Stmt::Force { lhs, rhs, .. } => {
                if let Some(r) = rhs {
                    self.reads_expr(r, sid, out);
                }
                self.reads_lvalue(lhs, sid, out);
            }
            Stmt::Block { stmts, .. } => {
                // block-local variables live in a child scope created at codegen time
                let bsid = self.code.block_scopes.get(&(sid, st as *const Stmt)).copied().unwrap_or(sid);
                if bsid != sid {
                    for s in self.scopes[bsid].syms.values() {
                        if let Sym::Sig(x) = s {
                            locals.push(*x);
                        }
                    }
                }
                for s in stmts {
                    self.reads_stmt_inner(s, bsid, out, locals);
                }
            }
            Stmt::Assign { lhs, rhs, .. } => {
                self.reads_expr(rhs, sid, out);
                self.reads_lvalue(lhs, sid, out);
            }
            Stmt::If { cond, then_s, else_s, .. } => {
                self.reads_expr(cond, sid, out);
                self.reads_stmt_inner(then_s, sid, out, locals);
                if let Some(e) = else_s {
                    self.reads_stmt_inner(e, sid, out, locals);
                }
            }
            Stmt::Case { expr, arms, default, .. } => {
                self.reads_expr(expr, sid, out);
                for (labels, b) in arms {
                    for l in labels {
                        self.reads_expr(l, sid, out);
                    }
                    self.reads_stmt_inner(b, sid, out, locals);
                }
                if let Some(d) = default {
                    self.reads_stmt_inner(d, sid, out, locals);
                }
            }
            Stmt::For { init, cond, step, body, .. } => {
                // the loop variable is written before it is read
                if let Stmt::Assign { lhs: Expr::Ident(n), .. } = &**init {
                    if let Some(Sym::Sig(s)) = self.lookup(n, sid) {
                        locals.push(s);
                    }
                }
                self.reads_stmt_inner(init, sid, out, locals);
                self.reads_expr(cond, sid, out);
                self.reads_stmt_inner(step, sid, out, locals);
                self.reads_stmt_inner(body, sid, out, locals);
            }
            Stmt::While { cond, body, .. } | Stmt::DoWhile { cond, body, .. } | Stmt::Wait { cond, body, .. } => {
                self.reads_expr(cond, sid, out);
                self.reads_stmt_inner(body, sid, out, locals);
            }
            Stmt::Repeat { count, body, .. } => {
                self.reads_expr(count, sid, out);
                self.reads_stmt_inner(body, sid, out, locals);
            }
            Stmt::Forever { body, .. } | Stmt::Timed { body, .. } => self.reads_stmt_inner(body, sid, out, locals),
            Stmt::SysTask { args, .. } => {
                for a in args.iter().flatten() {
                    self.reads_expr(a, sid, out);
                }
            }
            Stmt::MethodCall { obj, args, .. } => {
                self.reads_expr(obj, sid, out);
                for a in args {
                    self.reads_expr(a, sid, out);
                }
            }
            Stmt::StaticCall { args, .. } => {
                for a in args {
                    self.reads_expr(a, sid, out);
                }
            }
            Stmt::TaskCall { name, args, .. } => {
                // a void function called as a statement: like a call in an expression
                if self.subr.func_reads.get() && self.lookup_func(name, sid).is_some_and(|(f, _)| !f.is_task) {
                    self.reads_expr(&Expr::Call(name.clone(), args.clone()), sid, out);
                    return;
                }
                for a in args {
                    self.reads_expr(a, sid, out);
                }
            }
            Stmt::Return { value, .. } => {
                if let Some(v) = value {
                    self.reads_expr(v, sid, out);
                }
            }
        }
    }
}

impl<'a> Compiler<'a> {
    /// The imports of a block's scope `b`: `P` makes all names of package P visible (after the
    /// block's own), `P::x` the one name.
    fn block_imports(&mut self, imports: &[String], b: usize) -> R<()> {
        for i in imports {
            let (p, n) = match i.split_once("::") {
                Some((p, n)) => (p, Some(n)),
                None => (i.as_str(), None),
            };
            let Some(&ps) = self.hier.pkg_scopes.get(p) else { return self.err(format!("unknown package '{p}'")) };
            match n {
                None => self.scopes[b].imports.push(ps),
                Some(n) => {
                    self.scopes[b].name_imports.insert(n.to_string(), ps);
                }
            }
        }
        Ok(())
    }
}

impl<'a> Compiler<'a> {
    /// The parameters declared in a block or subroutine, in its scope `b`.
    pub(crate) fn local_params(&mut self, params: &[ParamDecl], b: usize) -> R<()> {
        for p in params {
            // (a `defparam` of it: registered during elaboration, before any code)
            let ov = match p.local {
                false => {
                    let key = format!("{}.{}", self.scopes[b].name, p.name);
                    let ov = self.hier.defparams.get(&key).cloned();
                    self.hier.params_done.insert(key);
                    ov
                }
                true => None,
            };
            let v = self.param_value(p, ov, b)?;
            self.insert_param(p, v, b)?;
            if p.range.is_some() && !p.real {
                let r = self.range_vals(&p.range, b)?;
                self.scopes[b].param_ranges.insert(p.name.clone(), r);
            }
        }
        Ok(())
    }
}
