//! Statement code generation.

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
        let (rw, rs) = self.ty(rhs, sid)?;
        let w = rw.max(lw);
        let v = self.expr(rhs, sid, w, rs)?;
        Ok(self.resize(v, w, lw, false))
    }

    /// Store targets of an lvalue, msb-first, with widths.
    fn targets(&mut self, e: &Expr, sid: usize, out: &mut Vec<(Option<Target>, u32)>) -> R<()> {
        match e {
            Expr::Concat(v) => {
                for x in v {
                    self.targets(x, sid, out)?;
                }
                Ok(())
            }
            Expr::Ident(_) | Expr::Hier(_) => match self.resolve_base(e, sid)? {
                Some(expr::Base::Sig(s)) => {
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
                        let e = self.const_int(i, sid)? - info.arr_lo;
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
                        let b = self.const_int(i, sid)?;
                        let off = if neg { k - b } else { b + k };
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
                    let b = self.const_int(i, sid)?;
                    let off = if neg { k - b } else { b + k };
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
                let (mv, lv) = (self.const_int(m, sid)?, self.const_int(l, sid)?);
                let pw = ((mv - lv).abs() + 1) as u32;
                if let Expr::Index(arr_e, ai) = &**a {
                    if let Some(arr) = self.array_of(arr_e, sid) {
                        let info = self.sig(arr).clone();
                        let ix = self.index_of(ai, sid, false, -info.arr_lo)?;
                        let (neg, k) = Self::bit_map(info.msb, info.lsb);
                        let off = if neg { k - lv } else { lv + k };
                        if off < 0 || off + pw as i64 > info.width as i64 {
                            return self.err("part select of array element out of range");
                        }
                        out.push((Some(Target::ElemPart { sig: arr, ix, lsb: off as u32, w: pw }), pw));
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
                let pw = self.const_int(width, sid)? as u32;
                let s = self.lsig(a, sid)?;
                let info = self.sig(s).clone();
                let (neg, k) = Self::index_part_map(info.msb, info.lsb, pw, *up);
                let ix = self.index_of(base, sid, neg, k)?;
                out.push((Some(Target::DynPart { sig: s, ix, w: pw }), pw));
                Ok(())
            }
            _ => self.err("invalid assignment target"),
        }
    }

    fn lsig(&mut self, e: &Expr, sid: usize) -> R<u32> {
        match self.resolve_base(e, sid)? {
            Some(expr::Base::Sig(s)) => Ok(s),
            _ => self.err("invalid assignment target"),
        }
    }

    fn lhs_targets(&mut self, lhs: &Lhs) -> R<Vec<(Option<Target>, u32)>> {
        match lhs {
            Lhs::Sig(s) => Ok(vec![(Some(Target::Whole { sig: *s }), self.sig(*s).width)]),
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
            Lhs::Expr(e, sid) => Ok(self.ty(e, *sid)?.0),
        }
    }

    /// Store a value (slot, width) into an lvalue.
    pub(crate) fn store_value(&mut self, lhs: &Lhs, slot: u32, w: u32, mode: StoreMode) -> R<()> {
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
        self.begin_proc(name, ProcClass::Normal);
        let top = self.here();
        let lw = self.lhs_width(&lhs)?;
        let mut reads = Vec::new();
        let v = match &rhs {
            Rhs::Expr(e, sid) => {
                let v = self.rhs_value(e, *sid, lw)?;
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
            let ds = self.expr(d, sid, 64, false)?;
            // copy the value before waiting
            let t = self.ptmp(lw);
            self.emit(Instr::Copy { d: t, s: v, w: lw });
            self.emit(Instr::Delay { s: ds });
            self.store_value(&lhs, t, lw, StoreMode::Blocking)?;
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

    fn event_ctl(&mut self, ec: &EventCtl, body: &Stmt, sid: usize) -> R<u32> {
        let mut trig = Vec::new();
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
                    match e {
                        Expr::Ident(_) | Expr::Hier(_) => match self.resolve_base(e, sid)? {
                            Some(expr::Base::Sig(s)) => trig.push((s, re, NONE)),
                            _ => return self.err("event expression is not a signal"),
                        },
                        _ => {
                            if re != REdge::Any {
                                self.warn("edge on a non-signal expression: triggering on any change");
                            }
                            let mut reads = Vec::new();
                            self.reads_expr(e, sid, &mut reads);
                            trig.extend(reads.into_iter().map(|(s, e)| (s, REdge::Any, e)));
                        }
                    }
                }
            }
        }
        Ok(self.make_ctl(trig))
    }

    fn timing(&mut self, t: &Timing, body: &Stmt, sid: usize) -> R<()> {
        match t {
            Timing::Delay(d) => {
                let ds = match d {
                    Expr::Real(r) => self.const_slot(&Const::from_i64(r.round() as i64, 64, false)),
                    _ => self.expr(d, sid, 64, false)?,
                };
                self.emit(Instr::Delay { s: ds });
            }
            Timing::Event(ec) => {
                let ctl = self.event_ctl(ec, body, sid)?;
                self.emit(Instr::Wait { ctl });
            }
        }
        Ok(())
    }

    pub(crate) fn stmt(&mut self, st: &Stmt, sid: usize) -> R<()> {
        match st {
            Stmt::Null => {}
            Stmt::Block { name, decls, stmts, fork } => {
                if *fork && stmts.len() > 1 {
                    self.warn("fork/join is executed sequentially (not supported yet)");
                }
                let bsid = if name.is_some() || !decls.is_empty() {
                    let path = match name {
                        Some(n) => format!("{}.{}", self.scopes[sid].name, n),
                        None => self.scopes[sid].name.clone(),
                    };
                    let file = self.file.clone();
                    let b = self.new_scope(path.clone(), Some(sid), file);
                    if let Some(n) = name {
                        self.scopes[sid].children.insert(n.clone(), b);
                    }
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
                            NetKind::Time => {
                                (m, l) = (63, 0);
                                SigKind::Reg
                            }
                            _ => SigKind::Reg,
                        };
                        let w = ((m - l).abs() + 1) as u32;
                        for dl in &d.names {
                            let (depth, lo) = if let Some((a, b2)) = dl.dims.first() {
                                let (a, b2) = (self.const_int(a, b)?, self.const_int(b2, b)?);
                                (((a - b2).abs() + 1) as u32, a.min(b2))
                            } else {
                                (0, 0)
                            };
                            let s =
                                self.new_signal(format!("{path}.{}", dl.name), w, signed, depth, m, l, lo, kind);
                            self.scopes[b].syms.insert(dl.name.clone(), Sym::Sig(s));
                            if let Some(init) = &dl.init {
                                let v = self.rhs_value(init, b, w)?;
                                self.emit(Instr::Store { t: Target::Whole { sig: s }, s: v, mode: StoreMode::Blocking });
                            }
                        }
                    }
                    b
                } else {
                    sid
                };
                if let Some(n) = name {
                    self.pb.last_mut().unwrap().blocks.push((n.clone(), Vec::new()));
                }
                for s in stmts {
                    self.stmt(s, bsid)?;
                }
                if name.is_some() {
                    let (_, patches) = self.pb.last_mut().unwrap().blocks.pop().unwrap();
                    let end = self.here();
                    for p in patches {
                        self.patch(p, end);
                    }
                }
            }
            Stmt::Assign { lhs, rhs, nonblocking, timing, line } => {
                self.line = *line;
                let lw = self.ty(lhs, sid)?.0;
                let v = self.rhs_value(rhs, sid, lw)?;
                let lv = Lhs::Expr(lhs, sid);
                match timing {
                    None => {
                        let mode = if *nonblocking { StoreMode::Nba } else { StoreMode::Blocking };
                        self.store_value(&lv, v, lw, mode)?;
                    }
                    Some(Timing::Delay(d)) if *nonblocking => {
                        let ds = self.expr(d, sid, 64, false)?;
                        self.store_value(&lv, v, lw, StoreMode::NbaDelay(ds))?;
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
                let (c, cw, _) = self.self_expr(cond, sid)?;
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
                let (mut w, mut s) = self.ty(expr, sid)?;
                for (labels, _) in arms {
                    for l in labels {
                        let (lw, ls) = self.ty(l, sid)?;
                        w = w.max(lw);
                        s &= ls;
                    }
                }
                let ev = self.expr(expr, sid, w, s)?;
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
                        let lv = self.expr(l, sid, w, s)?;
                        let t = self.tmp(1);
                        self.emit(Instr::Cmp { op, d: t, a: e, b: lv, w });
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
                let (c, cw, _) = self.self_expr(cond, sid)?;
                let jz = self.emit(Instr::Jz { c, w: cw, to: 0 });
                self.pb.last_mut().unwrap().loops.push((Vec::new(), Vec::new()));
                self.stmt(body, sid)?;
                let (brk, cont) = self.pb.last_mut().unwrap().loops.pop().unwrap();
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
                let (c, cw, _) = self.self_expr(cond, sid)?;
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
                self.pb.last_mut().unwrap().loops.push((Vec::new(), Vec::new()));
                self.stmt(body, sid)?;
                let ctop = self.here();
                let (c, cw, _) = self.self_expr(cond, sid)?;
                let jz = self.emit(Instr::Jz { c, w: cw, to: 0 });
                self.emit(Instr::Jmp { to: top });
                let h = self.here();
                self.patch(jz, h);
                let (brk, cont) = self.pb.last_mut().unwrap().loops.pop().unwrap();
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
                self.timing(timing, body, sid)?;
                self.stmt(body, sid)?;
            }
            Stmt::Wait { cond, body, line } => {
                self.line = *line;
                let top = self.here();
                let (c, cw, _) = self.self_expr(cond, sid)?;
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
            Stmt::TaskCall { name, args, line } => {
                self.line = *line;
                self.call(name, args, sid, true)?;
            }
            Stmt::Disable { name, line } => {
                self.line = *line;
                let found = self.pb.last().unwrap().blocks.iter().rposition(|b| &b.0 == name);
                match found {
                    Some(i) => {
                        let j = self.emit(Instr::Jmp { to: 0 });
                        self.pb.last_mut().unwrap().blocks[i].1.push(j);
                    }
                    None => self.warn(format!("disable of '{name}' outside the block is not supported")),
                }
            }
            Stmt::Trigger { line, .. } => {
                self.line = *line;
                return self.err("named events are not supported yet");
            }
            Stmt::Return { value, line } => {
                self.line = *line;
                let Some((ret, _)) = self.pb.last().unwrap().rets.last().cloned() else {
                    return self.err("return outside of a function or task");
                };
                if let (Some(v), Some(r)) = (value, ret) {
                    let w = self.sig(r).width;
                    let val = self.rhs_value(v, sid, w)?;
                    self.emit(Instr::Store { t: Target::Whole { sig: r }, s: val, mode: StoreMode::Blocking });
                }
                let j = self.emit(Instr::Jmp { to: 0 });
                self.pb.last_mut().unwrap().rets.last_mut().unwrap().1.push(j);
            }
            Stmt::Break { line } | Stmt::Continue { line } => {
                self.line = *line;
                let is_break = matches!(st, Stmt::Break { .. });
                let j = self.emit(Instr::Jmp { to: 0 });
                match self.pb.last_mut().unwrap().loops.last_mut() {
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
        self.pb.last_mut().unwrap().loops.push((Vec::new(), Vec::new()));
        self.stmt(body, sid)?;
        let cont = std::mem::take(&mut self.pb.last_mut().unwrap().loops.last_mut().unwrap().1);
        for j in cont {
            self.patch(j, cont_target);
        }
        Ok(())
    }

    fn finish_loop(&mut self, end: u32) {
        let (brk, _) = self.pb.last_mut().unwrap().loops.pop().unwrap();
        for j in brk {
            self.patch(j, end);
        }
    }

    fn fmt_args(&mut self, args: &[Option<Expr>], sid: usize) -> R<Vec<FmtArg>> {
        let mut v = Vec::new();
        for a in args {
            v.push(match a {
                None => FmtArg::Empty,
                Some(Expr::Str(s)) => FmtArg::Lit(s.clone()),
                Some(Expr::SysCall(n, _)) if n == "$time" || n == "$realtime" => {
                    let d = self.tmp(64);
                    self.emit(Instr::SysF { f: rt::SysFn::Time, d });
                    FmtArg::Time { s: d }
                }
                Some(e) => {
                    let (s, w, signed) = self.self_expr(e, sid)?;
                    FmtArg::Val { s, w, signed }
                }
            });
        }
        Ok(v)
    }

    fn sys_task(&mut self, name: &str, args: &[Option<Expr>], sid: usize) -> R<()> {
        let scope = self.scopes[sid].name.clone();
        let sys = |t: SysTask| Instr::Sys(Box::new(t));
        match name {
            "$display" | "$write" | "$displayb" | "$displayh" | "$displayo" | "$writeb" | "$writeh" | "$writeo" => {
                let args = self.fmt_args(args, sid)?;
                let nl = name.starts_with("$display");
                self.emit(sys(SysTask::Display { nl, args, scope, fd: None, prefix: "" }));
            }
            "$error" | "$warning" | "$info" | "$fatal" => {
                let mut a = args;
                if name == "$fatal" && !a.is_empty() && !matches!(a[0], Some(Expr::Str(_))) {
                    a = &a[1..];
                }
                let args = self.fmt_args(a, sid)?;
                let prefix = match name {
                    "$error" => "ERROR: ",
                    "$warning" => "WARNING: ",
                    "$info" => "INFO: ",
                    _ => "FATAL: ",
                };
                self.emit(sys(SysTask::Display { nl: true, args, scope, fd: None, prefix }));
                if name == "$fatal" {
                    self.emit(sys(SysTask::Finish));
                }
            }
            "$fdisplay" | "$fwrite" => {
                let Some(Some(fd)) = args.first() else { return self.err(format!("{name} needs a file descriptor")) };
                let (fs, fw, _) = self.self_expr(fd, sid)?;
                let args = self.fmt_args(&args[1..], sid)?;
                self.emit(sys(SysTask::Display { nl: name == "$fdisplay", args, scope, fd: Some((fs, fw)), prefix: "" }));
            }
            "$fclose" => {
                if let Some(Some(fd)) = args.first() {
                    let (fs, _, _) = self.self_expr(fd, sid)?;
                    self.emit(sys(SysTask::FClose { s: fs }));
                }
            }
            "$monitor" | "$strobe" => {
                let is_mon = name == "$monitor";
                let class = ProcClass::Postponed;
                let p = self.begin_proc(scope.clone(), class);
                let top = self.here();
                let fargs = self.fmt_args(args, sid)?;
                if is_mon {
                    self.emit(sys(SysTask::MonitorPrint { proc: p, args: fargs, scope: scope.clone() }));
                    let mut reads = Vec::new();
                    for e in args.iter().flatten() {
                        self.reads_expr(e, sid, &mut reads);
                    }
                    let ctl = self.make_ctl(reads.into_iter().map(|(s, e)| (s, REdge::Any, e)).collect());
                    self.emit(Instr::Wait { ctl });
                    self.emit(Instr::Jmp { to: top });
                } else {
                    self.emit(sys(SysTask::Display { nl: true, args: fargs, scope: scope.clone(), fd: None, prefix: "" }));
                    self.emit(Instr::Halt);
                }
                self.end_proc();
                if is_mon {
                    self.emit(sys(SysTask::Monitor { proc: p }));
                } else {
                    self.emit(sys(SysTask::Strobe { proc: p }));
                }
            }
            "$monitoron" => {
                self.emit(sys(SysTask::MonitorOn(true)));
            }
            "$monitoroff" => {
                self.emit(sys(SysTask::MonitorOn(false)));
            }
            "$finish" | "$stop" => {
                self.emit(sys(SysTask::Finish));
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
            "$dumpall" | "$dumpflush" | "$dumplimit" | "$timeformat" | "$printtimescale" => {}
            "$readmemh" | "$readmemb" => {
                let file = match args.first() {
                    Some(Some(Expr::Str(f))) => f.clone(),
                    _ => return self.err(format!("{name}: file name must be a string literal")),
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
                self.emit(sys(SysTask::ReadMem { hex: name == "$readmemh", file, sig, start, end }));
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
            Stmt::Null | Stmt::Disable { .. } | Stmt::Trigger { .. } | Stmt::Break { .. } | Stmt::Continue { .. } => {}
            Stmt::Block { stmts, name, .. } => {
                // block-local variables live in a child scope created at codegen time
                let bsid = name.as_ref().and_then(|n| self.scopes[sid].children.get(n).copied()).unwrap_or(sid);
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
            Stmt::TaskCall { args, .. } => {
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
