//! Expression typing (IEEE 1364 self-/context-determined widths) and code generation.

use crate::*;
use ferrooxide_front::ast::BinOp as B;
use ferrooxide_rt::{BinOp as RB, CmpOp, Index, RedOp, ShOp, SysFn, Target, UnOp as RU};

/// What an identifier-like base expression refers to.
pub(crate) enum Base {
    Sig(u32),
    Const(Const),
}

impl<'a> Compiler<'a> {
    pub(crate) fn resolve_base(&mut self, e: &Expr, sid: usize) -> R<Option<Base>> {
        let sym = match e {
            Expr::Ident(n) => match self.lookup(n, sid) {
                Some(s) => s,
                None => return self.err(format!("undeclared identifier '{n}'")),
            },
            Expr::Hier(p) => match self.lookup_hier(p, sid) {
                Some(s) => s,
                None => return self.err(format!("cannot resolve hierarchical name '{}'", p.join("."))),
            },
            _ => return Ok(None),
        };
        Ok(Some(match sym {
            Sym::Sig(s) => Base::Sig(s),
            Sym::Const(c) => Base::Const(c),
        }))
    }

    /// If `e` names an array, return its signal.
    pub(crate) fn array_of(&mut self, e: &Expr, sid: usize) -> Option<u32> {
        let sym = match e {
            Expr::Ident(n) => self.lookup(n, sid)?,
            Expr::Hier(p) => self.lookup_hier(p, sid)?,
            _ => return None,
        };
        match sym {
            Sym::Sig(s) if self.sig(s).depth > 0 => Some(s),
            _ => None,
        }
    }

    pub(crate) fn is_const(&self, e: &Expr, sid: usize) -> bool {
        match e {
            Expr::Num(_) | Expr::Str(_) | Expr::Real(_) => true,
            Expr::Ident(n) => matches!(self.lookup(n, sid), Some(Sym::Const(_))),
            Expr::Hier(_) => false,
            Expr::Index(a, b) => self.is_const(a, sid) && self.is_const(b, sid),
            Expr::Range(a, b, c) | Expr::Cond(a, b, c) => {
                self.is_const(a, sid) && self.is_const(b, sid) && self.is_const(c, sid)
            }
            Expr::IndexedRange(a, b, c, _) => self.is_const(a, sid) && self.is_const(b, sid) && self.is_const(c, sid),
            Expr::Unary(_, a) => self.is_const(a, sid),
            Expr::Binary(_, a, b) => self.is_const(a, sid) && self.is_const(b, sid),
            Expr::Concat(v) => v.iter().all(|x| self.is_const(x, sid)),
            Expr::Repl(n, v) => self.is_const(n, sid) && v.iter().all(|x| self.is_const(x, sid)),
            Expr::Call(..) => false,
            Expr::SysCall(n, args) => {
                matches!(n.as_str(), "$clog2" | "$signed" | "$unsigned" | "$bits")
                    && args.iter().all(|x| self.is_const(x, sid))
            }
        }
    }

    /// Bit-index mapping for a vector declared [msb:lsb]: offset = neg ? k - i : i + k
    pub(crate) fn bit_map(msb: i64, lsb: i64) -> (bool, i64) {
        if msb >= lsb {
            (false, -lsb)
        } else {
            (true, lsb)
        }
    }

    /// Value of a vector base for bit/part selection: (slot, width, msb, lsb)
    fn vec_base(&mut self, e: &Expr, sid: usize) -> R<(u32, u32, i64, i64)> {
        // array element as base: mem[i][3:0]
        if let Expr::Index(a, i) = e {
            if let Some(arr) = self.array_of(a, sid) {
                let info = self.sig(arr).clone();
                let ix = self.index_of(i, sid, false, -info.arr_lo)?;
                let d = self.tmp(info.width);
                self.emit(Instr::ArrRead { d, sig: arr, ix });
                return Ok((d, info.width, info.msb, info.lsb));
            }
        }
        match self.resolve_base(e, sid)? {
            Some(Base::Sig(s)) => {
                let info = self.sig(s).clone();
                if info.depth > 0 {
                    return self.err(format!("array '{}' used without index", info.name));
                }
                Ok((info.slot, info.width, info.msb, info.lsb))
            }
            Some(Base::Const(c)) => {
                let s = self.const_slot(&c);
                Ok((s, c.w, c.w as i64 - 1, 0))
            }
            None => {
                let (s, w, _) = self.self_expr(e, sid)?;
                Ok((s, w, w as i64 - 1, 0))
            }
        }
    }

    pub(crate) fn index_of(&mut self, e: &Expr, sid: usize, neg: bool, k: i64) -> R<Index> {
        let (slot, w, signed) = self.self_expr(e, sid)?;
        Ok(Index { slot, w, signed, neg, k })
    }

    // ------------------------------------------------------------------ typing
    pub(crate) fn ty(&mut self, e: &Expr, sid: usize) -> R<(u32, bool)> {
        Ok(match e {
            Expr::Num(n) => {
                if n.fill {
                    (1, false)
                } else {
                    let c = Const::from_number(n).or_else(|m| self.err(m))?;
                    (c.w, c.signed)
                }
            }
            Expr::Real(_) => return self.err("real numbers are not supported"),
            Expr::Str(s) => ((8 * s.len()).max(8) as u32, false),
            Expr::Ident(_) | Expr::Hier(_) => match self.resolve_base(e, sid)? {
                Some(Base::Sig(s)) => {
                    let i = self.sig(s);
                    (i.width, i.signed)
                }
                Some(Base::Const(c)) => (c.w, c.signed),
                None => unreachable!(),
            },
            Expr::Index(a, _) => {
                if let Some(arr) = self.array_of(a, sid) {
                    let i = self.sig(arr);
                    (i.width, i.signed)
                } else {
                    (1, false)
                }
            }
            Expr::Range(_, m, l) => {
                let (m, l) = (self.const_int(m, sid)?, self.const_int(l, sid)?);
                (((m - l).abs() + 1) as u32, false)
            }
            Expr::IndexedRange(_, _, w, _) => (self.const_int(w, sid)? as u32, false),
            Expr::Unary(op, a) => match op {
                UnOp::Plus | UnOp::Neg | UnOp::Not => self.ty(a, sid)?,
                _ => (1, false),
            },
            Expr::Binary(op, a, b) => match op {
                B::Add | B::Sub | B::Mul | B::Div | B::Mod | B::And | B::Or | B::Xor | B::Xnor => {
                    let (wa, sa) = self.ty(a, sid)?;
                    let (wb, sb) = self.ty(b, sid)?;
                    (wa.max(wb), sa && sb)
                }
                B::Pow | B::Shl | B::Shr | B::AShl | B::AShr => self.ty(a, sid)?,
                _ => (1, false),
            },
            Expr::Cond(_, t, f) => {
                let (wt, st) = self.ty(t, sid)?;
                let (wf, sf) = self.ty(f, sid)?;
                (wt.max(wf), st && sf)
            }
            Expr::Concat(v) => {
                let mut w = 0;
                for x in v {
                    w += self.ty(x, sid)?.0;
                }
                (w, false)
            }
            Expr::Repl(n, v) => {
                let n = self.const_int(n, sid)?;
                let mut w = 0;
                for x in v {
                    w += self.ty(x, sid)?.0;
                }
                ((w as i64 * n.max(0)) as u32, false)
            }
            Expr::Call(name, _) => {
                let Some((f, fs)) = self.lookup_func(name, sid) else {
                    return self.err(format!("unknown function '{name}'"));
                };
                if f.is_task {
                    return self.err(format!("task '{name}' used as a function"));
                }
                self.func_ret_type(f, fs)?
            }
            Expr::SysCall(name, args) => match name.as_str() {
                "$time" => (64, false),
                "$stime" => (32, false),
                "$realtime" => (64, false),
                "$random" => (32, true),
                "$urandom" | "$urandom_range" => (32, false),
                "$signed" | "$unsigned" => {
                    let a = args.first().ok_or_else(|| self.mkerr(format!("{name} needs an argument")))?;
                    (self.ty(a, sid)?.0, name == "$signed")
                }
                "$clog2" | "$bits" => (32, false),
                "$fopen" => (32, false),
                "$countones" => (32, false),
                "$onehot" | "$onehot0" | "$isunknown" => (1, false),
                _ => return self.err(format!("unsupported system function '{name}'")),
            },
        })
    }

    pub(crate) fn mkerr(&self, msg: String) -> ferrooxide_front::Error {
        ferrooxide_front::Error { msg, file: self.file.clone(), line: self.line }
    }

    pub(crate) fn func_ret_type(&mut self, f: &SubroutineDecl, fs: usize) -> R<(u32, bool)> {
        if f.ret_kind == NetKind::Integer {
            return Ok((32, true));
        }
        let (m, l) = self.range_vals(&f.range, fs)?;
        Ok((((m - l).abs() + 1) as u32, f.signed))
    }

    /// Compile in a self-determined context.
    pub(crate) fn self_expr(&mut self, e: &Expr, sid: usize) -> R<(u32, u32, bool)> {
        let (w, s) = self.ty(e, sid)?;
        let slot = self.expr(e, sid, w, s)?;
        Ok((slot, w, s))
    }

    /// Resize the value in `slot` from `sw` to `w`.
    pub(crate) fn resize(&mut self, slot: u32, sw: u32, w: u32, sign: bool) -> u32 {
        if sw == w {
            return slot;
        }
        let d = self.tmp(w);
        self.emit(Instr::Resize { d, s: slot, sw, dw: w, sign });
        d
    }

    /// Compile `e` to a value of width `w`, with expression signedness `s` (IEEE context propagation).
    pub(crate) fn expr(&mut self, e: &Expr, sid: usize, w: u32, s: bool) -> R<u32> {
        // constant folding
        if !self.pb.last().map(|p| p.proc == NONE).unwrap_or(true) && self.is_const(e, sid) {
            if !matches!(e, Expr::Num(_) | Expr::Ident(_)) {
                let c = self.const_eval(e, sid)?;
                let (_, own_s) = self.ty(e, sid)?;
                let c = c.resize(w, s && own_s || s && c.signed);
                return Ok(self.const_slot(&c));
            }
        }
        match e {
            Expr::Num(n) => {
                if n.fill {
                    let c = Const::fill(n.digits.chars().next().unwrap(), w);
                    return Ok(self.const_slot(&c));
                }
                let c = Const::from_number(n).or_else(|m| self.err(m))?;
                let c = c.resize(w, s);
                Ok(self.const_slot(&c))
            }
            Expr::Real(_) => self.err("real numbers are not supported"),
            Expr::Str(st) => {
                let c = Const::from_str_lit(st).resize(w, false);
                Ok(self.const_slot(&c))
            }
            Expr::Ident(_) | Expr::Hier(_) => match self.resolve_base(e, sid)? {
                Some(Base::Sig(sg)) => {
                    let i = self.sig(sg).clone();
                    if i.depth > 0 {
                        return self.err(format!("array '{}' used without index", i.name));
                    }
                    Ok(self.resize(i.slot, i.width, w, s))
                }
                Some(Base::Const(c)) => {
                    let c = c.resize(w, s);
                    Ok(self.const_slot(&c))
                }
                None => unreachable!(),
            },
            Expr::Index(a, i) => {
                if let Some(arr) = self.array_of(a, sid) {
                    let info = self.sig(arr).clone();
                    let ix = self.index_of(i, sid, false, -info.arr_lo)?;
                    let d = self.tmp(info.width);
                    self.emit(Instr::ArrRead { d, sig: arr, ix });
                    return Ok(self.resize(d, info.width, w, s));
                }
                let (bs, bw, msb, lsb) = self.vec_base(a, sid)?;
                let (neg, k) = Self::bit_map(msb, lsb);
                let d = self.tmp(1);
                if self.is_const(i, sid) {
                    let iv = self.const_eval(i, sid)?;
                    match iv.to_i64() {
                        Some(iv) => {
                            let off = if neg { k - iv } else { iv + k };
                            if off >= 0 && off < bw as i64 {
                                self.emit(Instr::Slice { d, s: bs, sw: bw, lsb: off as u32, w: 1 });
                            } else {
                                let c = self.const_slot(&Const::fill('x', 1));
                                return Ok(self.resize(c, 1, w, false));
                            }
                        }
                        None => {
                            let c = self.const_slot(&Const::fill('x', 1));
                            return Ok(self.resize(c, 1, w, false));
                        }
                    }
                } else {
                    let ix = self.index_of(i, sid, neg, k)?;
                    self.emit(Instr::DynSlice { d, s: bs, sw: bw, ix, w: 1 });
                }
                Ok(self.resize(d, 1, w, false))
            }
            Expr::Range(a, m, l) => {
                let (mv, lv) = (self.const_int(m, sid)?, self.const_int(l, sid)?);
                let (bs, bw, msb, lsb) = self.vec_base(a, sid)?;
                let (neg, k) = Self::bit_map(msb, lsb);
                let pw = ((mv - lv).abs() + 1) as u32;
                // the less significant end of the select is `lv` in a range of the same direction
                let off = if neg { k - lv } else { lv + k };
                let d = self.tmp(pw);
                if off >= 0 && off + pw as i64 <= bw as i64 {
                    self.emit(Instr::Slice { d, s: bs, sw: bw, lsb: off as u32, w: pw });
                } else {
                    let c = self.const_slot(&Const::from_i64(off, 64, true));
                    let ix = Index { slot: c, w: 64, signed: true, neg: false, k: 0 };
                    self.emit(Instr::DynSlice { d, s: bs, sw: bw, ix, w: pw });
                }
                Ok(self.resize(d, pw, w, false))
            }
            Expr::IndexedRange(a, base, width, up) => {
                let pw = self.const_int(width, sid)? as u32;
                let (bs, bw, msb, lsb) = self.vec_base(a, sid)?;
                let (neg, k) = Self::index_part_map(msb, lsb, pw, *up);
                let d = self.tmp(pw);
                let ix = self.index_of(base, sid, neg, k)?;
                self.emit(Instr::DynSlice { d, s: bs, sw: bw, ix, w: pw });
                Ok(self.resize(d, pw, w, false))
            }
            Expr::Unary(op, a) => match op {
                UnOp::Plus => self.expr(a, sid, w, s),
                UnOp::Neg | UnOp::Not => {
                    let v = self.expr(a, sid, w, s)?;
                    let d = self.tmp(w);
                    let op = if *op == UnOp::Neg { RU::Neg } else { RU::Not };
                    self.emit(Instr::Un { op, d, s: v, w });
                    Ok(d)
                }
                _ => {
                    let (v, vw, _) = self.self_expr(a, sid)?;
                    let op = match op {
                        UnOp::LNot => RedOp::Nor,
                        UnOp::RedAnd => RedOp::And,
                        UnOp::RedOr => RedOp::Or,
                        UnOp::RedXor => RedOp::Xor,
                        UnOp::RedNand => RedOp::Nand,
                        UnOp::RedNor => RedOp::Nor,
                        _ => RedOp::Xnor,
                    };
                    let d = self.tmp(1);
                    self.emit(Instr::Red { op, d, s: v, w: vw });
                    Ok(self.resize(d, 1, w, false))
                }
            },
            Expr::Binary(op, a, b) => self.binary(*op, a, b, sid, w, s),
            Expr::Cond(c, t, f) => {
                let (cv, cw, _) = self.self_expr(c, sid)?;
                let tv = self.expr(t, sid, w, s)?;
                let fv = self.expr(f, sid, w, s)?;
                let d = self.tmp(w);
                self.emit(Instr::Mux { d, c: cv, cw, t: tv, f: fv, w });
                Ok(d)
            }
            Expr::Concat(v) => {
                let mut parts = Vec::new();
                let mut tw = 0;
                for x in v {
                    let (xs, xw, _) = self.self_expr(x, sid)?;
                    if xw == 0 {
                        continue;
                    }
                    parts.push((xs, xw));
                    tw += xw;
                }
                let d = self.tmp(tw);
                self.emit(Instr::Concat { d, w: tw, parts: parts.into_boxed_slice() });
                Ok(self.resize(d, tw, w, false))
            }
            Expr::Repl(n, v) => {
                let n = self.const_int(n, sid)?;
                let mut one = Vec::new();
                for x in v {
                    let (xs, xw, _) = self.self_expr(x, sid)?;
                    one.push((xs, xw));
                }
                let mut parts = Vec::new();
                for _ in 0..n.max(0) {
                    parts.extend_from_slice(&one);
                }
                let tw: u32 = parts.iter().map(|p| p.1).sum();
                if tw == 0 {
                    return Ok(self.const_slot(&Const::zero(w)));
                }
                let d = self.tmp(tw);
                self.emit(Instr::Concat { d, w: tw, parts: parts.into_boxed_slice() });
                Ok(self.resize(d, tw, w, false))
            }
            Expr::Call(name, args) => {
                let (slot, rw, rs) = self.call(name, args, sid, false)?;
                let slot = slot.unwrap();
                // copy the result: the return variable is shared between calls
                let d = self.tmp(rw);
                self.emit(Instr::Copy { d, s: slot, w: rw });
                Ok(self.resize(d, rw, w, s && rs))
            }
            Expr::SysCall(name, args) => self.sys_func(name, args, sid, w, s),
        }
    }

    pub(crate) fn index_part_map(msb: i64, lsb: i64, pw: u32, up: bool) -> (bool, i64) {
        let pw = pw as i64;
        if msb >= lsb {
            if up {
                (false, -lsb)
            } else {
                (false, -lsb - pw + 1)
            }
        } else if up {
            (true, lsb - pw + 1)
        } else {
            (true, lsb)
        }
    }

    fn binary(&mut self, op: B, a: &Expr, b: &Expr, sid: usize, w: u32, s: bool) -> R<u32> {
        match op {
            B::Add | B::Sub | B::Mul | B::Div | B::Mod | B::And | B::Or | B::Xor | B::Xnor => {
                let av = self.expr(a, sid, w, s)?;
                let bv = self.expr(b, sid, w, s)?;
                let rop = match op {
                    B::Add => RB::Add,
                    B::Sub => RB::Sub,
                    B::Mul => RB::Mul,
                    B::Div => {
                        if s {
                            RB::DivS
                        } else {
                            RB::Div
                        }
                    }
                    B::Mod => {
                        if s {
                            RB::ModS
                        } else {
                            RB::Mod
                        }
                    }
                    B::And => RB::And,
                    B::Or => RB::Or,
                    B::Xor => RB::Xor,
                    _ => RB::Xnor,
                };
                let d = self.tmp(w);
                self.emit(Instr::Bin { op: rop, d, a: av, b: bv, w });
                Ok(d)
            }
            B::Pow => {
                let av = self.expr(a, sid, w, s)?;
                let (bv, bw, bs) = self.self_expr(b, sid)?;
                let d = self.tmp(w);
                self.emit(Instr::Pow { d, a: av, b: bv, w, bw, signed: bs });
                Ok(d)
            }
            B::Shl | B::AShl | B::Shr | B::AShr => {
                let av = self.expr(a, sid, w, s)?;
                let (bv, bw, _) = self.self_expr(b, sid)?;
                let sop = match op {
                    B::Shl | B::AShl => ShOp::Shl,
                    B::Shr => ShOp::Shr,
                    _ => {
                        if s {
                            ShOp::AShr
                        } else {
                            ShOp::Shr
                        }
                    }
                };
                let d = self.tmp(w);
                self.emit(Instr::Shift { op: sop, d, a: av, w, b: bv, bw });
                Ok(d)
            }
            B::LAnd | B::LOr => {
                let (av, aw, _) = self.self_expr(a, sid)?;
                let (bv, bw, _) = self.self_expr(b, sid)?;
                let ta = self.tmp(1);
                self.emit(Instr::Red { op: RedOp::Bool, d: ta, s: av, w: aw });
                let tb = self.tmp(1);
                self.emit(Instr::Red { op: RedOp::Bool, d: tb, s: bv, w: bw });
                let d = self.tmp(1);
                let rop = if op == B::LAnd { RB::And } else { RB::Or };
                self.emit(Instr::Bin { op: rop, d, a: ta, b: tb, w: 1 });
                Ok(self.resize(d, 1, w, false))
            }
            _ => {
                // comparisons
                let (wa, sa) = self.ty(a, sid)?;
                let (wb, sb) = self.ty(b, sid)?;
                let ow = wa.max(wb);
                let os = sa && sb;
                let av = self.expr(a, sid, ow, os)?;
                let bv = self.expr(b, sid, ow, os)?;
                let cop = match (op, os) {
                    (B::Eq, _) => CmpOp::Eq,
                    (B::Ne, _) => CmpOp::Ne,
                    (B::CaseEq, _) => CmpOp::CaseEq,
                    (B::CaseNe, _) => CmpOp::CaseNe,
                    (B::WildEq, _) => CmpOp::WildEq,
                    (B::WildNe, _) => CmpOp::WildNe,
                    (B::Lt, false) => CmpOp::Lt,
                    (B::Le, false) => CmpOp::Le,
                    (B::Gt, false) => CmpOp::Gt,
                    (B::Ge, false) => CmpOp::Ge,
                    (B::Lt, true) => CmpOp::LtS,
                    (B::Le, true) => CmpOp::LeS,
                    (B::Gt, true) => CmpOp::GtS,
                    (B::Ge, true) => CmpOp::GeS,
                    _ => unreachable!(),
                };
                let d = self.tmp(1);
                self.emit(Instr::Cmp { op: cop, d, a: av, b: bv, w: ow });
                Ok(self.resize(d, 1, w, false))
            }
        }
    }

    fn sys_func(&mut self, name: &str, args: &[Expr], sid: usize, w: u32, s: bool) -> R<u32> {
        match name {
            "$time" | "$stime" | "$realtime" => {
                let d = self.tmp(64);
                self.emit(Instr::SysF { f: SysFn::Time, d });
                let own = if name == "$stime" { 32 } else { 64 };
                let d = self.resize(d, 64, own, false);
                Ok(self.resize(d, own, w, false))
            }
            "$random" | "$urandom" => {
                let d = self.tmp(32);
                let f = match args.first() {
                    Some(Expr::Ident(n)) if name == "$random" => match self.lookup(n, sid) {
                        Some(Sym::Sig(sg)) => SysFn::RandomSeed(sg),
                        _ => return self.err("$random seed must be a variable"),
                    },
                    _ => {
                        if name == "$random" {
                            SysFn::Random
                        } else {
                            SysFn::Urandom
                        }
                    }
                };
                self.emit(Instr::SysF { f, d });
                Ok(self.resize(d, 32, w, s))
            }
            "$signed" | "$unsigned" => {
                let (v, vw, _) = self.self_expr(&args[0], sid)?;
                Ok(self.resize(v, vw, w, s))
            }
            "$clog2" => {
                let (v, vw, _) = self.self_expr(&args[0], sid)?;
                let d = self.tmp(32);
                self.emit(Instr::Clog2 { d, s: v, w: vw });
                Ok(self.resize(d, 32, w, false))
            }
            "$bits" => {
                let (bw, _) = self.ty(&args[0], sid)?;
                Ok(self.const_slot(&Const::from_i64(bw as i64, w, false)))
            }
            "$fopen" => {
                let file = match args.first() {
                    Some(Expr::Str(f)) => f.clone(),
                    _ => return self.err("$fopen needs a string file name"),
                };
                let d = self.tmp(32);
                self.emit(Instr::Sys(Box::new(rt::SysTask::FOpen { d, dw: 32, file })));
                Ok(self.resize(d, 32, w, false))
            }
            _ => self.err(format!("unsupported system function '{name}'")),
        }
    }

    // ------------------------------------------------------------------ functions & tasks
    pub(crate) fn func_instance(&mut self, f: &'a SubroutineDecl, def_scope: usize) -> R<()> {
        let key = (def_scope, f.name.clone());
        if self.func_inst.contains_key(&key) {
            return Ok(());
        }
        let path = format!("{}.{}", self.scopes[def_scope].name, f.name);
        let file = self.file.clone();
        let fs = self.new_scope(path.clone(), Some(def_scope), file);
        let ret = if f.is_task || f.ret_kind == NetKind::Int(0) {
            None
        } else {
            let (w, s) = self.func_ret_type(f, def_scope)?;
            let (m, l) = if f.ret_kind == NetKind::Integer { (31, 0) } else { self.range_vals(&f.range, def_scope)? };
            let sg = self.new_signal(path.clone(), w, s, 0, m, l, 0, SigKind::Reg);
            self.scopes[fs].syms.insert(f.name.clone(), Sym::Sig(sg));
            Some(sg)
        };
        let mut ports = Vec::new();
        for d in &f.decls {
            let (mut m, mut l) = self.range_vals(&d.range, fs)?;
            let mut signed = d.signed;
            match d.kind {
                NetKind::Integer => {
                    (m, l) = (31, 0);
                    signed = true;
                }
                NetKind::Int(n) if d.range.is_none() => {
                    (m, l) = (n as i64 - 1, 0);
                }
                NetKind::Time => (m, l) = (63, 0),
                _ => {}
            }
            let w = ((m - l).abs() + 1) as u32;
            for dl in &d.names {
                let (depth, lo) = if let Some((a, b)) = dl.dims.first() {
                    let (a, b) = (self.const_int(a, fs)?, self.const_int(b, fs)?);
                    (((a - b).abs() + 1) as u32, a.min(b))
                } else {
                    (0, 0)
                };
                let sg = self.new_signal(format!("{path}.{}", dl.name), w, signed, depth, m, l, lo, SigKind::Reg);
                self.scopes[fs].syms.insert(dl.name.clone(), Sym::Sig(sg));
                if let Some(dir) = d.dir {
                    ports.push((dir, sg));
                }
            }
        }
        self.func_inst.insert(key, FuncInst { scope: fs, ret, ports });
        Ok(())
    }

    /// Inline a function or task call. Returns (return slot, width, signed).
    pub(crate) fn call(
        &mut self,
        name: &str,
        args: &[Expr],
        sid: usize,
        as_task: bool,
    ) -> R<(Option<u32>, u32, bool)> {
        let Some((f, fs_def)) = self.lookup_func(name, sid) else {
            return self.err(format!("unknown {} '{name}'", if as_task { "task" } else { "function" }));
        };
        if self.call_depth > 32 {
            return self.err(format!("recursive call of '{name}' is not supported"));
        }
        self.func_instance(f, fs_def)?;
        let key = (fs_def, f.name.clone());
        let (fscope, ret, ports) = {
            let fi = &self.func_inst[&key];
            (fi.scope, fi.ret, fi.ports.clone())
        };
        if args.len() > ports.len() {
            return self.err(format!("too many arguments to '{name}'"));
        }
        // inputs
        for (i, a) in args.iter().enumerate() {
            let (dir, ps) = ports[i];
            if dir == Dir::Input || dir == Dir::Inout {
                let pw = self.sig(ps).width;
                let v = self.rhs_value(a, sid, pw)?;
                self.emit(Instr::Store { t: Target::Whole { sig: ps }, s: v, mode: rt::StoreMode::Blocking });
            }
        }
        self.call_depth += 1;
        self.pb.last_mut().unwrap().rets.push((ret, Vec::new()));
        let saved_file = self.file.clone();
        let r = self.stmt(&f.body, fscope);
        self.file = saved_file;
        let (_, patches) = self.pb.last_mut().unwrap().rets.pop().unwrap();
        self.call_depth -= 1;
        r?;
        let end = self.here();
        for p in patches {
            self.patch(p, end);
        }
        // outputs
        for (i, a) in args.iter().enumerate() {
            let (dir, ps) = ports[i];
            if dir == Dir::Output || dir == Dir::Inout {
                let info = self.sig(ps).clone();
                self.store_value(&stmt::Lhs::Expr(a, sid), info.slot, info.width, rt::StoreMode::Blocking)?;
            }
        }
        Ok(match ret {
            Some(r) => {
                let i = self.sig(r);
                (Some(i.slot), i.width, i.signed)
            }
            None => (None, 0, false),
        })
    }

    // ------------------------------------------------------------------ read sets
    pub(crate) fn reads_expr(&self, e: &Expr, sid: usize, out: &mut Vec<u32>) {
        match e {
            Expr::Ident(n) => {
                if let Some(Sym::Sig(s)) = self.lookup(n, sid) {
                    out.push(s)
                }
            }
            Expr::Hier(p) => {
                if let Some(Sym::Sig(s)) = self.lookup_hier(p, sid) {
                    out.push(s)
                }
            }
            Expr::Index(a, b) => {
                self.reads_expr(a, sid, out);
                self.reads_expr(b, sid, out);
            }
            Expr::Range(a, b, c) | Expr::Cond(a, b, c) | Expr::IndexedRange(a, b, c, _) => {
                self.reads_expr(a, sid, out);
                self.reads_expr(b, sid, out);
                self.reads_expr(c, sid, out);
            }
            Expr::Unary(_, a) => self.reads_expr(a, sid, out),
            Expr::Binary(_, a, b) => {
                self.reads_expr(a, sid, out);
                self.reads_expr(b, sid, out);
            }
            Expr::Concat(v) | Expr::Call(_, v) | Expr::SysCall(_, v) => {
                for x in v {
                    self.reads_expr(x, sid, out);
                }
                if let Expr::Call(name, _) = e {
                    // signals of the enclosing module read inside the function body
                    if let Some((f, fs)) = self.lookup_func(name, sid) {
                        let mut inner = Vec::new();
                        if let Some(fi) = self.func_inst.get(&(fs, f.name.clone())) {
                            self.reads_stmt(&f.body, fi.scope, &mut inner);
                            let own: Vec<u32> = fi.ports.iter().map(|p| p.1).chain(fi.ret).collect();
                            out.extend(inner.into_iter().filter(|s| !own.contains(s)));
                        }
                    }
                }
            }
            Expr::Repl(n, v) => {
                self.reads_expr(n, sid, out);
                for x in v {
                    self.reads_expr(x, sid, out);
                }
            }
            Expr::Num(_) | Expr::Str(_) | Expr::Real(_) => {}
        }
    }

    /// Signals read by an lvalue's index expressions.
    pub(crate) fn reads_lvalue(&self, e: &Expr, sid: usize, out: &mut Vec<u32>) {
        match e {
            Expr::Index(a, b) => {
                self.reads_lvalue(a, sid, out);
                self.reads_expr(b, sid, out);
            }
            Expr::Range(a, _, _) => self.reads_lvalue(a, sid, out),
            Expr::IndexedRange(a, b, _, _) => {
                self.reads_lvalue(a, sid, out);
                self.reads_expr(b, sid, out);
            }
            Expr::Concat(v) => {
                for x in v {
                    self.reads_lvalue(x, sid, out);
                }
            }
            _ => {}
        }
    }
}
