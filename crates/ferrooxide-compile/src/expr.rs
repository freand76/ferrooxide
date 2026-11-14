//! Expression typing (IEEE 1364 self-/context-determined widths) and code generation.

use crate::*;
use ferrooxide_front::ast::BinOp as B;
use ferrooxide_rt::{BinOp as RB, CmpOp, Index, RedOp, ShOp, SysFn, Target, UnOp as RU};

/// Whether an expression contains a (user) function call.
pub(crate) fn has_call(e: &Expr) -> bool {
    match e {
        Expr::Call(..) => true,
        Expr::Index(a, b) | Expr::Binary(_, a, b) => has_call(a) || has_call(b),
        Expr::Range(a, b, c) | Expr::Cond(a, b, c) | Expr::IndexedRange(a, b, c, _) => has_call(a) || has_call(b) || has_call(c),
        Expr::Unary(_, a) => has_call(a),
        Expr::Concat(v) | Expr::SysCall(_, v) => v.iter().any(has_call),
        Expr::Repl(n, v) => has_call(n) || v.iter().any(has_call),
        _ => false,
    }
}

/// What an identifier-like base expression refers to.
pub(crate) enum Base {
    Sig(u32),
    Const(Const),
    Real(f64),
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
            Sym::Real(r) => Base::Real(r),
            Sym::Type(_) => return self.err("a data type is not a value"),
        }))
    }

    /// If `e` names an array, return its signal.
    /// An index chain on a multi-dimensional array (`m[i][j]`, also with selects after it) as
    /// one index into the flat storage: `m[(i - lo0) * n1 + (j - lo1)]`.
    pub(crate) fn md_norm(&self, e: &Expr, sid: usize) -> Option<Expr> {
        if self.md_dims.is_empty() && self.packed_dims.is_empty() {
            return None;
        }
        let b = |x: Expr| Box::new(x);
        if let Some(n) = self.packed_slice(e, sid) {
            return Some(n);
        }
        match e {
            Expr::Range(a, m, l) => return Some(Expr::Range(b(self.md_norm(a, sid)?), m.clone(), l.clone())),
            Expr::IndexedRange(a, base, w, up) => {
                return Some(Expr::IndexedRange(b(self.md_norm(a, sid)?), base.clone(), w.clone(), *up))
            }
            Expr::Index(..) => {}
            _ => return None,
        }
        let mut idx = Vec::new();
        let mut cur = e;
        while let Expr::Index(a, i) = cur {
            idx.push(&**i);
            cur = a;
        }
        idx.reverse();
        // already flat: the base names the storage (`$flat:m`)
        let flat = match cur {
            Expr::Ident(n) if !n.starts_with("$flat:") => Expr::Ident(format!("$flat:{n}")),
            Expr::Hier(p) if !p.last()?.starts_with("$flat:") => {
                let mut p = p.clone();
                let l = p.last_mut()?;
                *l = format!("$flat:{l}");
                Expr::Hier(p)
            }
            _ => return None,
        };
        if let Some(e) = self.packed_norm(cur, &flat, &idx, sid) {
            return Some(e);
        }
        let s = self.array_of(cur, sid)?;
        let dims = self.md_dims.get(&s)?;
        if idx.len() < dims.len() {
            return None;
        }
        let num = |v: i64| -> Expr {
            let n = Expr::Num(ferrooxide_front::parser::dec(v.unsigned_abs()));
            if v < 0 {
                Expr::Unary(UnOp::Neg, b(n))
            } else {
                n
            }
        };
        let mut lin: Option<Expr> = None;
        for (k, &(lo, n)) in dims.iter().enumerate() {
            let term = Expr::Binary(BinOp::Sub, b(idx[k].clone()), b(num(lo)));
            lin = Some(match lin {
                None => term,
                Some(l) => Expr::Binary(BinOp::Add, b(Expr::Binary(BinOp::Mul, b(l), b(num(n as i64)))), b(term)),
            });
        }
        let mut out = Expr::Index(b(flat), b(lin.unwrap()));
        for i in &idx[dims.len()..] {
            out = Expr::Index(b(out), b((*i).clone()));
        }
        Some(out)
    }

    /// Selects of a vector with several packed dimensions: `x[i]` is a slice of the flat
    /// vector, `x[i][j]` (all dimensions) a bit, further indices select inside an element.
    fn packed_norm(&self, base: &Expr, flat: &Expr, idx: &[&Expr], sid: usize) -> Option<Expr> {
        let sym = match base {
            Expr::Ident(n) => self.lookup(n, sid)?,
            Expr::Hier(p) => self.lookup_hier(p, sid)?,
            _ => return None,
        };
        let Sym::Sig(s) = sym else { return None };
        let dims = self.packed_dims.get(&s)?;
        let b = |x: Expr| Box::new(x);
        let num = |v: i64| -> Expr {
            let n = Expr::Num(ferrooxide_front::parser::dec(v.unsigned_abs()));
            if v < 0 {
                Expr::Unary(UnOp::Neg, b(n))
            } else {
                n
            }
        };
        let k = idx.len().min(dims.len());
        let width = |d: &(i64, i64)| (d.0 - d.1).abs() + 1;
        let mut off: Option<Expr> = None;
        for t in 0..k {
            let (l, r) = dims[t];
            let stride: i64 = dims[t + 1..].iter().map(width).product();
            // position from the right end of the dimension
            let pos = if l >= r {
                Expr::Binary(BinOp::Sub, b(idx[t].clone()), b(num(r)))
            } else {
                Expr::Binary(BinOp::Sub, b(num(r)), b(idx[t].clone()))
            };
            let term = Expr::Binary(BinOp::Mul, b(pos), b(num(stride)));
            off = Some(match off {
                None => term,
                Some(o) => Expr::Binary(BinOp::Add, b(o), b(term)),
            });
        }
        let rem: i64 = dims[k..].iter().map(width).product();
        let mut out = if rem == 1 {
            Expr::Index(b(flat.clone()), b(off?))
        } else {
            Expr::IndexedRange(b(flat.clone()), b(off?), b(num(rem)), true)
        };
        for i in &idx[k..] {
            out = Expr::Index(b(out), b((*i).clone()));
        }
        Some(out)
    }

    /// `x[m:l]`, `x[b +: w]`, `x[b -: w]` on a vector with several packed dimensions: whole
    /// elements of the first dimension, as a part of the flat vector.
    fn packed_slice(&self, e: &Expr, sid: usize) -> Option<Expr> {
        let base = match e {
            Expr::Range(a, ..) | Expr::IndexedRange(a, ..) => &**a,
            _ => return None,
        };
        // a part of an element of the last dimension: x[i][m:l], x[i][b +: w]
        if let Expr::Index(..) = base {
            return self.packed_elem_part(e, base, sid);
        }
        let (flat, s) = match base {
            Expr::Ident(n) if !n.starts_with("$flat:") => match self.lookup(n, sid)? {
                Sym::Sig(s) => (Expr::Ident(format!("$flat:{n}")), s),
                _ => return None,
            },
            _ => return None,
        };
        let dims = self.packed_dims.get(&s)?;
        let ew: i64 = dims[1..].iter().map(|d| (d.0 - d.1).abs() + 1).product();
        let (l0, r0) = dims[0];
        let pos = |i: i64| if l0 >= r0 { i - r0 } else { r0 - i };
        let b = |x: Expr| Box::new(x);
        let num = |v: i64| -> Expr {
            let n = Expr::Num(ferrooxide_front::parser::dec(v.unsigned_abs()));
            if v < 0 {
                Expr::Unary(UnOp::Neg, b(n))
            } else {
                n
            }
        };
        match e {
            Expr::Range(_, m, l) => {
                let (m, l) = (self.const_value(m, sid)?, self.const_value(l, sid)?);
                let lo = pos(m).min(pos(l));
                let n = (m - l).abs() + 1;
                Some(Expr::IndexedRange(b(flat), b(num(lo * ew)), b(num(n * ew)), true))
            }
            Expr::IndexedRange(_, bx, w, up) => {
                let w = self.const_value(w, sid)?;
                // the lowest position of the selected elements
                let first = match (l0 >= r0, *up) {
                    (true, true) => Expr::Binary(BinOp::Sub, bx.clone(), b(num(r0))),
                    (true, false) => Expr::Binary(BinOp::Sub, bx.clone(), b(num(r0 + w - 1))),
                    (false, true) => Expr::Binary(BinOp::Sub, b(num(r0 - w + 1)), bx.clone()),
                    (false, false) => Expr::Binary(BinOp::Sub, b(num(r0)), bx.clone()),
                };
                let off = Expr::Binary(BinOp::Mul, b(first), b(num(ew)));
                Some(Expr::IndexedRange(b(flat), b(off), b(num(w * ew)), true))
            }
            _ => None,
        }
    }

    /// `x[i][m:l]` / `x[i][b +: w]` where `x[i]` is an element of the last packed dimension.
    fn packed_elem_part(&self, e: &Expr, inner: &Expr, sid: usize) -> Option<Expr> {
        let mut idx = Vec::new();
        let mut cur = inner;
        while let Expr::Index(a, i) = cur {
            idx.push(&**i);
            cur = a;
        }
        idx.reverse();
        let Expr::Ident(n) = cur else { return None };
        if n.starts_with("$flat:") {
            return None;
        }
        let Sym::Sig(s) = self.lookup(n, sid)? else { return None };
        let dims = self.packed_dims.get(&s)?;
        if idx.len() + 1 != dims.len() {
            return None;
        }
        // the element's offset (as an expression), then the part inside it
        let Some(Expr::IndexedRange(flat, off, _, _)) = self.md_norm(inner, sid) else { return None };
        let (l, r) = dims[dims.len() - 1];
        let pos = |i: i64| if l >= r { i - r } else { r - i };
        let b = |x: Expr| Box::new(x);
        let num = |v: i64| -> Expr {
            let n = Expr::Num(ferrooxide_front::parser::dec(v.unsigned_abs()));
            if v < 0 {
                Expr::Unary(UnOp::Neg, b(n))
            } else {
                n
            }
        };
        match e {
            Expr::Range(_, m, lx) => {
                let (m, lx) = (self.const_value(m, sid)?, self.const_value(lx, sid)?);
                let lo = pos(m).min(pos(lx));
                let off = Expr::Binary(BinOp::Add, off, b(num(lo)));
                Some(Expr::IndexedRange(flat, b(off), b(num((m - lx).abs() + 1)), true))
            }
            Expr::IndexedRange(_, bx, w, up) => {
                let w = self.const_value(w, sid)?;
                let first = match (l >= r, *up) {
                    (true, true) => Expr::Binary(BinOp::Sub, bx.clone(), b(num(r))),
                    (true, false) => Expr::Binary(BinOp::Sub, bx.clone(), b(num(r + w - 1))),
                    (false, true) => Expr::Binary(BinOp::Sub, b(num(r - w + 1)), bx.clone()),
                    (false, false) => Expr::Binary(BinOp::Sub, b(num(r)), bx.clone()),
                };
                let off = Expr::Binary(BinOp::Add, off, b(first));
                Some(Expr::IndexedRange(flat, b(off), b(num(w)), true))
            }
            _ => None,
        }
    }

    pub(crate) fn array_of(&self, e: &Expr, sid: usize) -> Option<u32> {
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
            Expr::Type(_) => false,
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
                matches!(n.as_str(), "$clog2" | "$signed" | "$unsigned" | "$bits" | "$__cast" | "$__str")
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
                let (m, l) = match e {
                    Expr::Ident(n) => self.param_range(n, sid).unwrap_or((c.w as i64 - 1, 0)),
                    _ => (c.w as i64 - 1, 0),
                };
                Ok((s, c.w, m, l))
            }
            Some(Base::Real(_)) => self.err("bit select of a real value"),
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
        if let Some(n) = self.md_norm(e, sid) {
            return self.ty(&n, sid);
        }
        if self.is_real(e, sid) {
            // converted to a 64-bit integer where an integer is needed
            return Ok((64, true));
        }
        Ok(match e {
            Expr::Type(_) => return self.err("a data type is not a value"),
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
                Some(Base::Real(_)) => (64, true),
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
            Expr::Range(_, m, l) => match (self.const_index(m, sid)?, self.const_index(l, sid)?) {
                (Some(m), Some(l)) => (((m - l).abs() + 1) as u32, false),
                // an x/z bound: a 1-bit x
                _ => (1, false),
            },
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
                "$sqrt" | "$ln" | "$log10" | "$exp" | "$floor" | "$ceil" | "$sin" | "$cos" | "$tan" | "$asin" | "$acos" | "$atan" | "$sinh" | "$cosh" | "$tanh" | "$asinh" | "$acosh" | "$atanh" | "$atan2" | "$hypot" | "$pow" => (64, false),
                "$random" => (32, true),
                "$urandom" | "$urandom_range" => (32, false),
                "$signed" | "$unsigned" => {
                    let a = args.first().ok_or_else(|| self.mkerr(format!("{name} needs an argument")))?;
                    (self.ty(a, sid)?.0, name == "$signed")
                }
                "$clog2" | "$bits" => (32, false),
                // SV cast T'(e) / N'(e): (width, sign: 0, 1, 2 = the expression's, e)
                "$__cast" => {
                    let w = self.const_int(&args[0], sid)?;
                    if !(1..=1 << 20).contains(&w) {
                        return self.err(format!("bad cast width {w}"));
                    }
                    let sg = match self.const_int(&args[1], sid)? {
                        0 => false,
                        1 => true,
                        _ => self.ty(&args[2], sid)?.1,
                    };
                    (w as u32, sg)
                }
                "$__pattern" => return self.err("an assignment pattern needs a known target type here"),
                "$__str" => self.ty(&args[0], sid)?,
                "$fopen" => (32, false),
                "$test$plusargs" | "$value$plusargs" => (32, true),
                "$fgetc" | "$ungetc" | "$fgets" | "$feof" | "$ftell" | "$fseek" | "$rewind" | "$fscanf" | "$sscanf" => {
                    (32, true)
                }
                "$rtoi" => (32, true),
                "$realtobits" => (64, false),
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
        if f.ret_kind == NetKind::Real {
            return Ok((64, true));
        }
        let (m, l) = self.range_vals(&f.range, fs)?;
        Ok((((m - l).abs() + 1) as u32, f.signed))
    }

    /// Compile in a self-determined context.
    /// A value used as a condition: (slot, width); a real is true when it is not 0.0.
    pub(crate) fn truth(&mut self, e: &Expr, sid: usize) -> R<(u32, u32)> {
        if self.is_real(e, sid) {
            let r = self.rexpr(e, sid)?;
            let z = self.const_slot(&Const::zero(64));
            let d = self.tmp(1);
            self.emit(Instr::RCmp { op: CmpOp::Ne, d, a: r, b: z });
            return Ok((d, 1));
        }
        let (v, w, _) = self.self_expr(e, sid)?;
        Ok((v, w))
    }

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
        if let Some(n) = self.md_norm(e, sid) {
            return self.expr(&n, sid, w, s);
        }
        // a real expression where an integer is needed: round
        if self.is_real(e, sid) {
            let v = self.rexpr(e, sid)?;
            let d = self.tmp(w);
            self.emit(Instr::RToI { d, s: v, w, trunc: false });
            return Ok(d);
        }
        // constant folding
        if !self.pb.last().map(|p| p.proc == NONE).unwrap_or(true) && self.is_const(e, sid) {
            if !matches!(e, Expr::Num(_) | Expr::Ident(_)) {
                // evaluate in the context width and signedness (operands are extended first)
                self.pb.push(PState { proc: NONE, ..Default::default() });
                let r = self.expr(e, sid, w, s);
                let st = self.pb.pop().unwrap();
                let slot = r?;
                if !st.code.is_empty() {
                    if let Err(m) = rt::vm::eval_const(&st.code, &mut self.prog.mem, &self.prog.signals) {
                        return self.err(m);
                    }
                }
                let c = self.read_const(slot, w, s);
                return Ok(self.const_slot(&c));
            }
        }
        match e {
            Expr::Type(_) => self.err("a data type is not a value"),
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
                Some(Base::Real(_)) => unreachable!("real handled above"),
                Some(Base::Const(c)) => {
                    let c = c.resize(w, s);
                    Ok(self.const_slot(&c))
                }
                None => unreachable!(),
            },
            Expr::Index(a, i) => {
                if let Some(arr) = self.array_of(a, sid) {
                    let info = self.sig(arr).clone();
                    if self.is_const(i, sid) {
                        let e = self.const_int(i, sid)? - info.arr_lo;
                        if e < 0 || e >= info.depth as i64 {
                            let c = self.const_slot(&Const::fill('x', info.width));
                            return Ok(self.resize(c, info.width, w, s));
                        }
                        let es = info.slot + e as u32 * rt::words(info.width);
                        return Ok(self.resize(es, info.width, w, s));
                    }
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
                let (Some(mv), Some(lv)) = (self.const_index(m, sid)?, self.const_index(l, sid)?) else {
                    // an x/z bound: all x (also in a wider context, like Icarus)
                    let n = rt::nw(w);
                    let mut a = vec![u64::MAX; n];
                    if w % 64 != 0 {
                        a[n - 1] = (1u64 << (w % 64)) - 1;
                    }
                    let x = Const { w, signed: false, a: a.clone(), b: a };
                    return Ok(self.const_slot(&x));
                };
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
                    let (v, vw) = if *op == UnOp::LNot { self.truth(a, sid)? } else { self.self_expr(a, sid).map(|x| (x.0, x.1))? };
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
                let (cv, cw) = self.truth(c, sid)?;
                if has_call(t) || has_call(f) {
                    // branches with function calls (recursion!): only the selected side runs;
                    // an unknown condition evaluates both and blends them
                    let d = self.tmp(w);
                    let not_true = self.emit(Instr::Jz { c: cv, w: cw, to: 0 });
                    let tv = self.expr(t, sid, w, s)?;
                    self.emit(Instr::Copy { d, s: tv, w });
                    let end1 = self.emit(Instr::Jmp { to: 0 });
                    let h = self.here();
                    self.patch(not_true, h);
                    let nc = self.tmp(1);
                    self.emit(Instr::Red { op: RedOp::Nor, d: nc, s: cv, w: cw });
                    let unknown = self.emit(Instr::Jz { c: nc, w: 1, to: 0 });
                    let fv = self.expr(f, sid, w, s)?;
                    self.emit(Instr::Copy { d, s: fv, w });
                    let end2 = self.emit(Instr::Jmp { to: 0 });
                    let h = self.here();
                    self.patch(unknown, h);
                    let tv = self.expr(t, sid, w, s)?;
                    let fv = self.expr(f, sid, w, s)?;
                    self.emit(Instr::Mux { d, c: cv, cw, t: tv, f: fv, w });
                    let h = self.here();
                    self.patch(end1, h);
                    self.patch(end2, h);
                    return Ok(d);
                }
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
                self.emit_concat(d, tw, &parts);
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
                self.emit_concat(d, tw, &parts);
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

    /// Concatenation (parts msb-first). Two ≤64-bit parts use one Concat; otherwise the value is
    /// built from zero with one Insert per part, which the fast paths handle at any width.
    pub(crate) fn emit_concat(&mut self, d: u32, tw: u32, parts: &[(u32, u32)]) {
        if tw <= 64 && parts.len() == 2 {
            self.emit(Instr::Concat { d, w: tw, parts: parts.to_vec().into_boxed_slice() });
            return;
        }
        let zero = self.const_slot(&Const::zero(tw));
        self.emit(Instr::Copy { d, s: zero, w: tw });
        let mut off = 0;
        for &(s, pw) in parts.iter().rev() {
            self.emit(Instr::Insert { d, dw: tw, s, sw: pw, off });
            off += pw;
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
                let (av, aw) = self.truth(a, sid)?;
                let (bv, bw) = self.truth(b, sid)?;
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
                if self.is_real(a, sid) || self.is_real(b, sid) {
                    let (ra, rb) = (self.rexpr(a, sid)?, self.rexpr(b, sid)?);
                    let cop = match op {
                        B::Eq | B::CaseEq | B::WildEq => CmpOp::Eq,
                        B::Ne | B::CaseNe | B::WildNe => CmpOp::Ne,
                        B::Lt => CmpOp::Lt,
                        B::Le => CmpOp::Le,
                        B::Gt => CmpOp::Gt,
                        _ => CmpOp::Ge,
                    };
                    let d = self.tmp(1);
                    self.emit(Instr::RCmp { op: cop, d, a: ra, b: rb });
                    return Ok(self.resize(d, 1, w, false));
                }
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

    /// File input functions; returns the slot of the 32-bit result.
    pub(crate) fn file_read(&mut self, name: &str, args: &[Expr], sid: usize) -> R<u32> {
        use rt::FileRead as F;
        let min = match name {
            "$fgetc" | "$feof" | "$ftell" | "$rewind" => 1,
            "$fseek" => 3,
            _ => 2,
        };
        if args.len() < min {
            return self.err(format!("{name} needs {min} arguments"));
        }
        let fd_at = |me: &mut Self, i: usize| -> R<(u32, u32)> { Ok((me.expr(&args[i], sid, 32, false)?, 32)) };
        let scope = self.scopes[sid].name.clone();
        let d = self.tmp(32);
        let sys = |kind, src| Instr::Sys(Box::new(rt::SysTask::FileRead { kind, d, src, scope: scope.clone() }));
        match name {
            "$fgetc" | "$feof" | "$ftell" | "$rewind" => {
                let src = fd_at(self, 0)?;
                let kind = match name {
                    "$fgetc" => F::Getc,
                    "$feof" => F::Eof,
                    "$ftell" => F::Tell,
                    _ => F::Rewind,
                };
                self.emit(sys(kind, src));
            }
            "$ungetc" => {
                let c = self.expr(&args[0], sid, 32, false)?;
                let src = fd_at(self, 1)?;
                self.emit(sys(F::Ungetc { c }, src));
            }
            "$fseek" => {
                let src = fd_at(self, 0)?;
                let off = self.expr(&args[1], sid, 32, true)?;
                let op = self.expr(&args[2], sid, 32, false)?;
                self.emit(sys(F::Seek { off, op }, src));
            }
            "$fgets" => {
                let vw = self.ty(&args[0], sid)?.0;
                let val = self.tmp(vw);
                let src = fd_at(self, 1)?;
                self.emit(sys(F::Gets { val, w: vw }, src));
                let jz = self.emit(Instr::Jz { c: d, w: 32, to: 0 });
                self.store_value(&stmt::Lhs::Expr(&args[0], sid), val, vw, rt::StoreMode::Blocking)?;
                let h = self.here();
                self.patch(jz, h);
            }
            _ => {
                // $fscanf(fd, fmt, vars...) / $sscanf(str, fmt, vars...)
                let string = name == "$sscanf";
                let src = if string {
                    let (s, w, _) = self.self_expr(&args[0], sid)?;
                    (s, w)
                } else {
                    fd_at(self, 0)?
                };
                let Expr::Str(fmt) = &args[1] else {
                    return self.err(format!("{name}: the format must be a string literal"));
                };
                let mut outs = Vec::new();
                for v in &args[2..] {
                    let real = self.lvalue_is_real(v, sid);
                    let w = if real { 64 } else { self.ty(v, sid)?.0 };
                    let val = self.tmp(w);
                    let ok = self.tmp(1);
                    outs.push(rt::ScanOut { val, w, real, ok });
                }
                self.emit(sys(F::Scan { string, fmt: fmt.clone(), outs: outs.clone() }, src));
                // assign the variables that got a value
                for (v, o) in args[2..].iter().zip(&outs) {
                    let jz = self.emit(Instr::Jz { c: o.ok, w: 1, to: 0 });
                    self.store_value(&stmt::Lhs::Expr(v, sid), o.val, o.w, rt::StoreMode::Blocking)?;
                    let h = self.here();
                    self.patch(jz, h);
                }
            }
        }
        Ok(d)
    }

    fn sys_func(&mut self, name: &str, args: &[Expr], sid: usize, w: u32, s: bool) -> R<u32> {
        match name {
            "$time" | "$stime" => {
                let d = self.tmp(64);
                let e = self.texp(sid);
                self.emit(Instr::SysF { f: SysFn::Time(e), d });
                let own = if name == "$stime" { 32 } else { 64 };
                let d = self.resize(d, 64, own, false);
                Ok(self.resize(d, own, w, false))
            }
            "$random" | "$urandom" => {
                let d = self.tmp(32);
                let f = match args.first() {
                    Some(Expr::Ident(n)) => match self.lookup(n, sid) {
                        Some(Sym::Sig(sg)) if name == "$random" => SysFn::RandomSeed(sg),
                        Some(Sym::Sig(sg)) => SysFn::UrandomSeed(sg),
                        _ => return self.err(format!("{name} seed must be a variable")),
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
            "$urandom_range" => {
                let Some(max) = args.first() else { return self.err("$urandom_range needs a maximum") };
                let a = self.expr(max, sid, 32, false)?;
                let a = {
                    // keep it: the minimum's evaluation may reuse scratch slots
                    let k = self.tmp(32);
                    self.emit(Instr::Copy { d: k, s: a, w: 32 });
                    k
                };
                let b = match args.get(1) {
                    Some(e) => self.expr(e, sid, 32, false)?,
                    None => self.const_slot(&Const::from_i64(0, 32, false)),
                };
                let d = self.tmp(32);
                self.emit(Instr::SysF { f: SysFn::UrandomRange(a, b), d });
                Ok(self.resize(d, 32, w, s))
            }
            "$signed" | "$unsigned" => {
                let (v, vw, _) = self.self_expr(&args[0], sid)?;
                Ok(self.resize(v, vw, w, s))
            }
            "$__str" => self.expr(&args[0], sid, w, s),
            // any bit x or z: the reduction xor is x exactly then
            "$isunknown" => {
                let Some(a) = args.first() else { return self.err("$isunknown needs an argument") };
                let x = Expr::Num(ferrooxide_front::ast::Number { size: Some(1), signed: false, base: 2, based: true, digits: "x".into(), fill: false });
                let e = Expr::Binary(BinOp::CaseEq, Box::new(Expr::Unary(UnOp::RedXor, Box::new(a.clone()))), Box::new(x));
                self.expr(&e, sid, w, s)
            }
            "$__cast" => {
                // the operand is evaluated as if assigned to a variable of the cast's width
                let (cw, _) = self.ty(&Expr::SysCall(name.into(), args.to_vec()), sid)?;
                let (ew, es) = if self.is_real(&args[2], sid) { (cw, true) } else { self.ty(&args[2], sid)? };
                let ctx = cw.max(ew);
                let v = self.expr(&args[2], sid, ctx, es)?;
                let v = self.resize(v, ctx, cw, false);
                Ok(self.resize(v, cw, w, s))
            }
            "$clog2" => {
                let (v, vw, _) = self.self_expr(&args[0], sid)?;
                let d = self.tmp(32);
                self.emit(Instr::Clog2 { d, s: v, w: vw });
                Ok(self.resize(d, 32, w, false))
            }
            "$bits" => {
                // a type parameter
                if let Some(Expr::Ident(n)) = args.first() {
                    if let Some(Sym::Type(t)) = self.lookup(n, sid) {
                        let bw = self.const_int(&t.bits(), sid)?;
                        return Ok(self.const_slot(&Const::from_i64(bw, w, false)));
                    }
                }
                let (bw, _) = self.ty(&args[0], sid)?;
                Ok(self.const_slot(&Const::from_i64(bw as i64, w, false)))
            }
            "$fopen" => {
                let file = match args.first() {
                    Some(e) => self.file_name(e, sid)?,
                    None => return self.err("$fopen needs a file name"),
                };
                let mode = match args.get(1) {
                    Some(Expr::Str(m)) => m.clone(),
                    Some(_) => return self.err("$fopen mode must be a string literal"),
                    None => "w".to_string(),
                };
                let d = self.tmp(32);
                self.emit(Instr::Sys(Box::new(rt::SysTask::FOpen { d, dw: 32, file, mode })));
                Ok(self.resize(d, 32, w, false))
            }
            "$test$plusargs" => {
                let Some(Expr::Str(prefix)) = args.first() else {
                    return self.err("$test$plusargs needs a string");
                };
                let d = self.tmp(1);
                self.emit(Instr::Sys(Box::new(rt::SysTask::TestPlusargs { d, prefix: prefix.clone() })));
                Ok(self.resize(d, 1, w, false))
            }
            "$value$plusargs" => {
                let (Some(Expr::Str(f)), Some(var)) = (args.first(), args.get(1)) else {
                    return self.err("$value$plusargs needs a format string and a variable");
                };
                let Some(pct) = f.find('%') else {
                    return self.err("$value$plusargs format has no conversion");
                };
                let prefix = f[..pct].to_string();
                let conv = f[pct + 1..].chars().find(|c| c.is_ascii_alphabetic()).unwrap_or('d').to_ascii_lowercase();
                let real = self.lvalue_is_real(var, sid);
                let vw = if real { 64 } else { self.ty(var, sid)?.0 };
                let val = self.tmp(vw);
                let d = self.tmp(1);
                self.emit(Instr::Sys(Box::new(rt::SysTask::ValuePlusargs { d, prefix, conv, val, w: vw, real })));
                // assign the variable only when the plusarg was found
                let jz = self.emit(Instr::Jz { c: d, w: 1, to: 0 });
                self.store_value(&stmt::Lhs::Expr(var, sid), val, vw, rt::StoreMode::Blocking)?;
                let h = self.here();
                self.patch(jz, h);
                Ok(self.resize(d, 1, w, false))
            }
            "$fgetc" | "$ungetc" | "$fgets" | "$feof" | "$ftell" | "$fseek" | "$rewind" | "$fscanf" | "$sscanf" => {
                let d = self.file_read(name, args, sid)?;
                Ok(self.resize(d, 32, w, true))
            }
            "$rtoi" => {
                let v = self.rexpr(&args[0], sid)?;
                let d = self.tmp(32);
                self.emit(Instr::RToI { d, s: v, w: 32, trunc: true });
                Ok(self.resize(d, 32, w, true))
            }
            "$realtobits" => {
                let v = self.rexpr(&args[0], sid)?;
                let d = self.tmp(64);
                self.emit(Instr::Copy { d, s: v, w: 64 });
                Ok(self.resize(d, 64, w, false))
            }
            _ => self.err(format!("unsupported system function '{name}'")),
        }
    }

    // ------------------------------------------------------------------ functions & tasks
    /// Storage for a function/task: shared (static), or per call site for `automatic` ones
    /// (`key` distinguishes the instances).
    pub(crate) fn func_instance(&mut self, f: &'a SubroutineDecl, def_scope: usize, key: &str) -> R<()> {
        let key = (def_scope, key.to_string());
        if self.func_inst.contains_key(&key) {
            return Ok(());
        }
        let mem_lo = self.prog.mem.len() as u32;
        let path = format!("{}.{}", self.scopes[def_scope].name, f.name);
        let file = self.file.clone();
        let fs = self.new_scope(path.clone(), Some(def_scope), file);
        let ret = if f.is_task || f.ret_kind == NetKind::Int(0) {
            None
        } else {
            let (w, s) = self.func_ret_type(f, def_scope)?;
            let (m, l) = match f.ret_kind {
                NetKind::Integer => (31, 0),
                NetKind::Real => (63, 0),
                _ => self.range_vals(&f.range, def_scope)?,
            };
            let sg = self.new_signal(path.clone(), w, s, 0, m, l, 0, SigKind::Reg);
            if f.ret_kind == NetKind::Real {
                self.mark_real(sg);
            }
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
                NetKind::Time | NetKind::Real => (m, l) = (63, 0),
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
                if d.kind == NetKind::Real {
                    self.mark_real(sg);
                }
                if d.kind == NetKind::Event {
                    self.zero_init(sg);
                }
                self.mark_two_state(sg, d.kind);
                self.scopes[fs].syms.insert(dl.name.clone(), Sym::Sig(sg));
                if let Some(dir) = d.dir {
                    ports.push((dir, sg));
                }
            }
        }
        if key.1 == f.name {
            // the static instance: reachable by hierarchical names (task.var)
            self.scopes[def_scope].children.entry(f.name.clone()).or_insert(fs);
        }
        let mem = (mem_lo, self.prog.mem.len() as u32);
        self.func_inst.insert(key, FuncInst { scope: fs, ret, ports, mem });
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
        // a call of a function that is being inlined: recursion, through a subroutine
        if let Some(k) = self.inline_stack.iter().rev().find(|x| x.0 == fs_def && x.1 == f.name).map(|x| x.2.clone()) {
            return self.recursive_call(f, k, args, sid);
        }
        if self.call_depth > 32 {
            return self.err(format!("calls nested too deeply in '{name}'"));
        }
        let kname = if f.automatic {
            self.auto_count += 1;
            format!("{}#{}", f.name, self.auto_count)
        } else {
            f.name.clone()
        };
        self.func_instance(f, fs_def, &kname)?;
        let key = (fs_def, kname);
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
                let real = self.sig(ps).real;
                let v = self.value_for(a, sid, pw, real)?;
                self.emit(Instr::Store { t: Target::Whole { sig: ps }, s: v, mode: rt::StoreMode::Blocking });
            }
        }
        self.call_depth += 1;
        self.inline_stack.push((fs_def, f.name.clone(), key.clone()));
        self.pb.last_mut().unwrap().rets.push((ret, Vec::new()));
        let saved_file = self.file.clone();
        // the body is a block `disable <task>` can end
        let bpath = format!("{}.{}", self.scopes[fs_def].name, f.name);
        let reg = self.open_block(bpath);
        let r = self.stmt(&f.body, fscope);
        self.close_block(reg);
        self.file = saved_file;
        let (_, patches) = self.pb.last_mut().unwrap().rets.pop().unwrap();
        self.call_depth -= 1;
        self.inline_stack.pop();
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

    /// A recursive call: save the function's variables (and temporaries), pass the arguments,
    /// jump to the function's subroutine; results come back in fresh temporaries.
    fn recursive_call(&mut self, f: &'a SubroutineDecl, key: (usize, String), args: &[Expr], sid: usize) -> R<(Option<u32>, u32, bool)> {
        let (ret, ports) = {
            let fi = &self.func_inst[&key];
            (fi.ret, fi.ports.clone())
        };
        if args.len() > ports.len() {
            return self.err(format!("too many arguments to '{}'", f.name));
        }
        // argument values first (they may read the current frame)
        let mut vals = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let (dir, ps) = ports[i];
            if dir == Dir::Input || dir == Dir::Inout {
                let pw = self.sig(ps).width;
                let real = self.sig(ps).real;
                let v = self.value_for(a, sid, pw, real)?;
                let k = self.tmp(pw);
                self.emit(Instr::Copy { d: k, s: v, w: pw });
                vals.push((ps, k));
            }
        }
        let level = self.pb.len();
        // recursion from another process (a fork branch inside the function) would need frames
        // shared between processes
        if self.subs.iter().any(|r| r.key == key && r.level != level) {
            return self.err(format!("recursive call of '{}' from a fork branch is not supported", f.name));
        }
        let si = match self.subs.iter().position(|r| r.level == level && r.key == key) {
            Some(i) => i,
            None => {
                self.subs.push(SubReq { level, f, key: key.clone(), calls: Vec::new(), saves: Vec::new(), entry: None, alloc: (0, 0) });
                self.subs.len() - 1
            }
        };
        let save = self.emit(Instr::SaveFrame { ranges: Box::new([]) });
        for (ps, k) in vals {
            self.emit(Instr::Store { t: Target::Whole { sig: ps }, s: k, mode: rt::StoreMode::Blocking });
        }
        // results: the return value, then output ports
        let mut outs: Vec<(u32, u32)> = Vec::new();
        if let Some(r) = ret {
            outs.push((r, self.sig(r).width));
        }
        for (i, a) in args.iter().enumerate() {
            let (dir, ps) = ports[i];
            if dir == Dir::Output || dir == Dir::Inout {
                let _ = a;
                outs.push((ps, self.sig(ps).width));
            }
        }
        let dsts: Vec<u32> = outs.iter().map(|&(_, w)| self.tmp(w)).collect();
        let call = self.emit(Instr::CallSub { entry: 0, dsts: dsts.clone().into_boxed_slice() });
        self.subs[si].saves.push(save);
        self.subs[si].calls.push(call);
        // outputs to the caller's arguments
        let mut k = if ret.is_some() { 1 } else { 0 };
        for (i, a) in args.iter().enumerate() {
            let (dir, ps) = ports[i];
            if dir == Dir::Output || dir == Dir::Inout {
                let w = self.sig(ps).width;
                self.store_value(&stmt::Lhs::Expr(a, sid), dsts[k], w, rt::StoreMode::Blocking)?;
                k += 1;
            }
        }
        Ok(match ret {
            Some(r) => {
                let i = self.sig(r);
                (Some(dsts[0]), i.width, i.signed)
            }
            None => (None, 0, false),
        })
    }

    /// Compile the subroutines of recursive functions the current process calls (after its
    /// code) and complete their calls and saved frames.
    pub(crate) fn finish_subs(&mut self) -> R<()> {
        let level = self.pb.len();
        if !self.subs.iter().any(|r| r.level == level) {
            return Ok(());
        }
        // the process's own code ends before the subroutines
        self.emit(Instr::Halt);
        while let Some(i) = self.subs.iter().position(|r| r.level == level && r.entry.is_none()) {
            let (f, key) = (self.subs[i].f, self.subs[i].key.clone());
            let (fscope, ret, ports, _) = {
                let fi = &self.func_inst[&key];
                (fi.scope, fi.ret, fi.ports.clone(), fi.mem)
            };
            let entry = self.here();
            self.subs[i].entry = Some(entry as usize);
            let a0 = self.prog.mem.len() as u32;
            self.call_depth += 1;
            self.inline_stack.push((key.0, f.name.clone(), key.clone()));
            self.pb.last_mut().unwrap().rets.push((ret, Vec::new()));
            let bpath = format!("{}.{}", self.scopes[key.0].name, f.name);
            let reg = self.open_block(bpath);
            let r = self.stmt(&f.body, fscope);
            // (on an error the compilation stops: the state is not cleaned up)
            r?;
            self.close_block(reg);
            let (_, patches) = self.pb.last_mut().unwrap().rets.pop().unwrap();
            self.inline_stack.pop();
            self.call_depth -= 1;
            let end = self.here();
            for p in patches {
                self.patch(p, end);
            }
            let mut srcs: Vec<(u32, u32)> = Vec::new();
            if let Some(r) = ret {
                let s = self.sig(r);
                srcs.push((s.slot, 2 * rt::nw(s.width) as u32));
            }
            for &(dir, ps) in &ports {
                if dir == Dir::Output || dir == Dir::Inout {
                    let s = self.sig(ps);
                    srcs.push((s.slot, 2 * rt::nw(s.width) as u32));
                }
            }
            self.emit(Instr::RetSub { srcs: srcs.into_boxed_slice() });
            self.subs[i].alloc = (a0, self.prog.mem.len() as u32);
        }
        // complete calls and frames: the function's variables, what its body allocated, and the
        // process's temporaries
        let hw = self.scratch_top;
        let reqs: Vec<usize> = (0..self.subs.len()).filter(|&i| self.subs[i].level == level).collect();
        for i in reqs.iter().rev() {
            let r = self.subs.remove(*i);
            let fm = self.func_inst[&r.key].mem;
            let mut ranges = vec![(fm.0, fm.1 - fm.0), (r.alloc.0, r.alloc.1 - r.alloc.0)];
            if hw > 0 {
                ranges.push((0, hw));
            }
            ranges.retain(|x| x.1 > 0);
            let code = &mut self.pb.last_mut().unwrap().code;
            for &c in &r.calls {
                if let Instr::CallSub { entry, .. } = &mut code[c] {
                    *entry = r.entry.unwrap() as u32;
                }
            }
            for &sv in &r.saves {
                code[sv] = Instr::SaveFrame { ranges: ranges.clone().into_boxed_slice() };
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------ read sets
    pub(crate) fn reads_expr(&self, e: &Expr, sid: usize, out: &mut Vec<(u32, u32)>) {
        if let Some(n) = self.md_norm(e, sid) {
            return self.reads_expr(&n, sid, out);
        }
        match e {
            Expr::Type(_) => {}
            Expr::Ident(n) => {
                if let Some(Sym::Sig(s)) = self.lookup(n, sid) {
                    out.push((s, NONE))
                }
            }
            Expr::Hier(p) => {
                if let Some(Sym::Sig(s)) = self.lookup_hier(p, sid) {
                    out.push((s, NONE))
                }
            }
            Expr::Index(a, b) => {
                // a constant-index array element is its own node for sensitivity
                if let Some(arr) = self.array_of(a, sid) {
                    if self.is_const(b, sid) {
                        let info = self.sig(arr);
                        if let Some(e) = self.const_value(b, sid) {
                            let e = e - info.arr_lo;
                            if e >= 0 && e < info.depth as i64 {
                                out.push((arr, e as u32));
                                return;
                            }
                        }
                    }
                }
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
                // signals a function reads besides its arguments: they do not count for
                // sensitivity (Icarus's continuous assignments and @* follow the arguments
                // only), but the lint check (synthesis view) includes them
                if let (Expr::Call(name, _), true) = (e, self.func_reads.get()) {
                    if let Some((f, fs)) = self.lookup_func(name, sid) {
                        // (functions calling each other: each is followed once)
                        let key = (fs, f.name.clone());
                        if !self.func_reads_seen.borrow().contains(&key) {
                            self.func_reads_seen.borrow_mut().push(key.clone());
                            let mut inner = Vec::new();
                            if let Some(fi) = self.func_inst.get(&key) {
                                self.reads_stmt(&f.body, fi.scope, &mut inner);
                                let own: Vec<u32> = fi.ports.iter().map(|p| p.1).chain(fi.ret).collect();
                                out.extend(inner.into_iter().filter(|s| !own.contains(&s.0)));
                            } else {
                                // an automatic function: its own variables do not resolve in its
                                // module, the module's signals it reads do
                                self.reads_stmt(&f.body, fs, &mut inner);
                                out.extend(inner);
                            }
                            self.func_reads_seen.borrow_mut().pop();
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
    pub(crate) fn reads_lvalue(&self, e: &Expr, sid: usize, out: &mut Vec<(u32, u32)>) {
        if let Some(n) = self.md_norm(e, sid) {
            return self.reads_lvalue(&n, sid, out);
        }
        match e {
            Expr::Index(a, b) => {
                if !matches!(&**a, Expr::Ident(_) | Expr::Hier(_)) {
                    self.reads_lvalue(a, sid, out);
                }
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

    // ------------------------------------------------------------------ real numbers
    /// Whether an expression has type real (IEEE: an operand of an arithmetic operator being real
    /// makes the operation real; comparisons and logical operators give integers).
    pub(crate) fn is_real(&self, e: &Expr, sid: usize) -> bool {
        if let Some(n) = self.md_norm(e, sid) {
            return self.is_real(&n, sid);
        }
        match e {
            Expr::Real(_) => true,
            Expr::Ident(_) | Expr::Hier(_) => {
                let sym = match e {
                    Expr::Ident(n) => self.lookup(n, sid),
                    Expr::Hier(p) => self.lookup_hier(p, sid),
                    _ => None,
                };
                match sym {
                    Some(Sym::Sig(s)) => self.sig(s).real,
                    Some(Sym::Real(_)) => true,
                    _ => false,
                }
            }
            Expr::Index(a, _) => self.array_of(a, sid).map(|s| self.sig(s).real).unwrap_or(false),
            Expr::Unary(UnOp::Plus | UnOp::Neg, a) => self.is_real(a, sid),
            Expr::Binary(B::Add | B::Sub | B::Mul | B::Div | B::Pow, a, b) => {
                self.is_real(a, sid) || self.is_real(b, sid)
            }
            Expr::Cond(_, t, f) => self.is_real(t, sid) || self.is_real(f, sid),
            Expr::Call(name, _) => self.lookup_func(name, sid).map(|(f, _)| f.ret_kind == NetKind::Real).unwrap_or(false),
            Expr::SysCall(name, _) => matches!(name.as_str(), "$realtime" | "$itor" | "$bitstoreal" | "$sqrt" | "$ln" | "$log10" | "$exp" | "$floor" | "$ceil" | "$sin" | "$cos" | "$tan" | "$asin" | "$acos" | "$atan" | "$sinh" | "$cosh" | "$tanh" | "$asinh" | "$acosh" | "$atanh" | "$atan2" | "$hypot" | "$pow"),
            _ => false,
        }
    }

    /// Whether an assignment target is a real variable.
    pub(crate) fn lvalue_is_real(&self, e: &Expr, sid: usize) -> bool {
        if let Some(n) = self.md_norm(e, sid) {
            return self.lvalue_is_real(&n, sid);
        }
        match e {
            Expr::Ident(_) | Expr::Hier(_) | Expr::Index(..) => self.is_real(e, sid),
            _ => false,
        }
    }

    /// Compile an expression as a real value: slot holding f64 bits (unknown word 0).
    pub(crate) fn rexpr(&mut self, e: &Expr, sid: usize) -> R<u32> {
        if let Some(n) = self.md_norm(e, sid) {
            return self.rexpr(&n, sid);
        }
        if !self.is_real(e, sid) {
            let (s, w, signed) = self.self_expr(e, sid)?;
            let d = self.tmp(64);
            self.emit(Instr::IToR { d, s, w, signed });
            return Ok(d);
        }
        let rconst = |c: &mut Self, r: f64| c.const_slot(&Const { w: 64, signed: false, a: vec![r.to_bits()], b: vec![0] });
        match e {
            Expr::Real(r) => Ok(rconst(self, *r)),
            Expr::Ident(_) | Expr::Hier(_) => match self.resolve_base(e, sid)? {
                Some(Base::Sig(s)) => Ok(self.sig(s).slot),
                Some(Base::Real(r)) => Ok(rconst(self, r)),
                _ => self.err("not a real value"),
            },
            Expr::Index(a, i) => {
                let arr = self.array_of(a, sid).unwrap();
                let info = self.sig(arr).clone();
                if self.is_const(i, sid) {
                    let e = self.const_int(i, sid)? - info.arr_lo;
                    if e >= 0 && e < info.depth as i64 {
                        return Ok(info.slot + e as u32 * rt::words(64));
                    }
                    return Ok(rconst(self, 0.0));
                }
                let ix = self.index_of(i, sid, false, -info.arr_lo)?;
                let d = self.tmp(64);
                self.emit(Instr::ArrRead { d, sig: arr, ix });
                Ok(d)
            }
            Expr::Unary(UnOp::Plus, a) => self.rexpr(a, sid),
            Expr::Unary(_, a) => {
                let v = self.rexpr(a, sid)?;
                let d = self.tmp(64);
                self.emit(Instr::RNeg { d, s: v });
                Ok(d)
            }
            Expr::Binary(op, a, b) => {
                let (va, vb) = (self.rexpr(a, sid)?, self.rexpr(b, sid)?);
                let rop = match op {
                    B::Add => rt::RealOp::Add,
                    B::Sub => rt::RealOp::Sub,
                    B::Mul => rt::RealOp::Mul,
                    B::Div => rt::RealOp::Div,
                    _ => rt::RealOp::Pow,
                };
                let d = self.tmp(64);
                self.emit(Instr::RBin { op: rop, d, a: va, b: vb });
                Ok(d)
            }
            Expr::Cond(c, t, f) => {
                let (cv, cw) = self.truth(c, sid)?;
                let (tv, fv) = (self.rexpr(t, sid)?, self.rexpr(f, sid)?);
                let d = self.tmp(64);
                self.emit(Instr::Mux { d, c: cv, cw, t: tv, f: fv, w: 64 });
                Ok(d)
            }
            Expr::Call(name, args) => {
                let (slot, _, _) = self.call(name, args, sid, false)?;
                let d = self.tmp(64);
                self.emit(Instr::Copy { d, s: slot.unwrap(), w: 64 });
                Ok(d)
            }
            Expr::SysCall(name, args) => match name.as_str() {
                "$realtime" => {
                    let d = self.tmp(64);
                    let e = self.texp(sid);
                    self.emit(Instr::SysF { f: SysFn::RealTime(e), d });
                    Ok(d)
                }
                "$sqrt" | "$ln" | "$log10" | "$exp" | "$floor" | "$ceil" | "$sin" | "$cos" | "$tan" | "$asin" | "$acos" | "$atan" | "$sinh" | "$cosh" | "$tanh" | "$asinh" | "$acosh" | "$atanh" | "$atan2" | "$hypot" | "$pow" => {
                    let op = match name.as_str() {
                        "$sqrt" => rt::RealOp::Sqrt,
                        "$ln" => rt::RealOp::Ln,
                        "$log10" => rt::RealOp::Log10,
                        "$exp" => rt::RealOp::Exp,
                        "$floor" => rt::RealOp::Floor,
                        "$ceil" => rt::RealOp::Ceil,
                        "$sin" => rt::RealOp::Sin,
                        "$cos" => rt::RealOp::Cos,
                        "$tan" => rt::RealOp::Tan,
                        "$asin" => rt::RealOp::Asin,
                        "$acos" => rt::RealOp::Acos,
                        "$atan" => rt::RealOp::Atan,
                        "$sinh" => rt::RealOp::Sinh,
                        "$cosh" => rt::RealOp::Cosh,
                        "$tanh" => rt::RealOp::Tanh,
                        "$asinh" => rt::RealOp::Asinh,
                        "$acosh" => rt::RealOp::Acosh,
                        "$atanh" => rt::RealOp::Atanh,
                        "$atan2" => rt::RealOp::Atan2,
                        "$hypot" => rt::RealOp::Hypot,
                        "$pow" => rt::RealOp::Pow,
                        _ => unreachable!(),
                    };
                    let two = matches!(op, rt::RealOp::Atan2 | rt::RealOp::Hypot | rt::RealOp::Pow);
                    if args.len() != 1 + two as usize {
                        return self.err(format!("{name} needs {} argument(s)", 1 + two as usize));
                    }
                    let a = self.rexpr(&args[0], sid)?;
                    let a = {
                        // keep it while the second argument is evaluated
                        let k = self.tmp(64);
                        self.emit(Instr::Copy { d: k, s: a, w: 64 });
                        k
                    };
                    let b = if two { self.rexpr(&args[1], sid)? } else { a };
                    let d = self.tmp(64);
                    self.emit(Instr::RBin { op, d, a, b });
                    Ok(d)
                }
                "$itor" => {
                    let a = args.first().ok_or_else(|| self.mkerr("$itor needs an argument".into()))?;
                    let (s, w, signed) = self.self_expr(a, sid)?;
                    let d = self.tmp(64);
                    self.emit(Instr::IToR { d, s, w, signed });
                    Ok(d)
                }
                _ => {
                    // $bitstoreal
                    let a = args.first().ok_or_else(|| self.mkerr(format!("{name} needs an argument")))?;
                    let v = self.expr(a, sid, 64, false)?;
                    let d = self.tmp(64);
                    self.emit(Instr::Copy { d, s: v, w: 64 });
                    Ok(d)
                }
            },
            _ => self.err("unsupported real expression"),
        }
    }

    /// Evaluate a constant real expression at elaboration time.
    pub(crate) fn const_real(&mut self, e: &Expr, sid: usize) -> R<f64> {
        if let Expr::Real(r) = e {
            return Ok(*r);
        }
        self.pb.push(PState { proc: NONE, ..Default::default() });
        let r = self.rexpr(e, sid);
        let st = self.pb.pop().unwrap();
        let slot = r?;
        if !st.code.is_empty() {
            if let Err(m) = rt::vm::eval_const(&st.code, &mut self.prog.mem, &self.prog.signals) {
                return self.err(m);
            }
        }
        Ok(f64::from_bits(self.prog.mem[slot as usize]))
    }

    /// The value for an assignment to a target that is real or `lw` bits wide.
    pub(crate) fn value_for(&mut self, rhs: &Expr, sid: usize, lw: u32, real: bool) -> R<u32> {
        if real {
            self.rexpr(rhs, sid)
        } else {
            self.rhs_value(rhs, sid, lw)
        }
    }

    /// A delay in the scope's time unit converted to simulation ticks (slot of a 64-bit value).
    pub(crate) fn delay_ticks(&mut self, d: &Expr, sid: usize) -> R<u32> {
        if let Expr::SysCall(n, v) = d {
            if n == "$__delays" || n == "$__pathdelays" {
                // (rise, fall, ...) where one delay is needed: the first
                return self.delay_ticks(&v[0], sid);
            }
        }
        let exp = self.texp(sid);
        // a real delay is rounded to the module's precision, then scaled to simulation ticks
        let sc = &self.scopes[sid];
        let to_prec = (sc.tunit - sc.tprec).clamp(0, 18) as u8;
        let post = (sc.tprec - self.gprec).clamp(0, 18) as u8;
        if self.is_real(d, sid) {
            if self.is_const(d, sid) {
                let r = self.const_real(d, sid)? * 10f64.powi(to_prec as i32);
                let t = if r > 0.0 { r.round() as i64 } else { 0 };
                let t = t * 10i64.pow(post as u32);
                return Ok(self.const_slot(&Const::from_i64(t, 64, false)));
            }
            let v = self.rexpr(d, sid)?;
            let t = self.tmp(64);
            self.emit(Instr::RToTicks { d: t, s: v, exp: to_prec, post });
            return Ok(t);
        }
        let v = self.expr(d, sid, 64, false)?;
        if exp == 0 {
            return Ok(v);
        }
        let k = self.const_slot(&Const::from_i64(10i64.pow(exp as u32), 64, false));
        let t = self.tmp(64);
        self.emit(Instr::Bin { op: rt::BinOp::Mul, d: t, a: v, b: k, w: 64 });
        Ok(t)
    }

    /// Rise, fall and turn-off delays (ticks) as one 192-bit value for `StoreMode::Inertial`;
    /// a missing turn-off delay is all ones (the runtime takes the smaller of rise and fall).
    pub(crate) fn delays3(&mut self, d: &Expr, sid: usize) -> R<u32> {
        let (ds, lsb_rule): (Vec<Expr>, bool) = match d {
            Expr::SysCall(n, v) if n == "$__delays" => (v.clone(), false),
            Expr::SysCall(n, v) if n == "$__pathdelays" => (v.clone(), true),
            _ => (vec![d.clone()], false),
        };
        let out = self.tmp(256);
        let mut init = Const::zero(256);
        if lsb_rule {
            init.a[3] = 1;
        }
        let z = self.const_slot(&init);
        self.emit(Instr::Copy { d: out, s: z, w: 256 });
        for k in 0..3 {
            let t = match ds.get(k).or(if ds.len() == 1 { ds.first() } else { None }) {
                Some(e) => self.delay_ticks(e, sid)?,
                None => self.const_slot(&Const::from_i64(-1, 64, true)),
            };
            self.emit(Instr::Insert { d: out, dw: 256, s: t, sw: 64, off: 64 * k as u32 });
        }
        Ok(out)
    }

    /// A file name argument: a string literal, or a variable holding the characters.
    pub(crate) fn file_name(&mut self, e: &Expr, sid: usize) -> R<rt::FileName> {
        if let Expr::Str(s) = e {
            return Ok(rt::FileName::Lit(s.clone()));
        }
        let (s, w, _) = self.self_expr(e, sid)?;
        Ok(rt::FileName::Val { s, w })
    }
}
