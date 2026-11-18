//! Expression typing (IEEE 1364 self-/context-determined widths) and code generation.

use crate::types::{Packed, Type};
use crate::*;
use ferrooxide_front::ast::BinOp as B;
use ferrooxide_rt::{BinOp as RB, CmpOp, Index, RedOp, ShOp, SysFn, Target, UnOp as RU};

/// Whether an expression contains a (user) function or method call or an increment: it may
/// have side effects, so only what the operators select may run.
pub(crate) fn has_call(e: &Expr) -> bool {
    match e {
        Expr::Call(..) | Expr::MethodCall(..) | Expr::IncDec { .. } => true,
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
        self.norm(e, sid, false)
    }

    /// `md_norm` of an assignment target (a signed struct member stays a plain select).
    pub(crate) fn lv_norm(&self, e: &Expr, sid: usize) -> Option<Expr> {
        self.norm(e, sid, true)
    }

    /// A select chain in the form the code generation knows: struct members, several
    /// unpacked or packed dimensions and `.triggered` resolved.
    fn norm(&self, e: &Expr, sid: usize, lvalue: bool) -> Option<Expr> {
        // `q.size` of a queue: a call
        if self.sigs.has_dyn {
            if let Some(c) = self.dyn_hier_call(e, sid) {
                return Some(c);
            }
        }
        if self.sigs.has_structs && matches!(e, Expr::Index(..) | Expr::Range(..) | Expr::IndexedRange(..) | Expr::Member(..)) {
            if let Some(Ok(w)) = self.struct_walk(e, sid, lvalue) {
                return Some(w.expr);
            }
        }
        // `ev.triggered` (IEEE 1800 15.5.3): the event was triggered in this time step
        if self.sigs.has_events {
            if let Expr::Hier(p) = e {
                if p.len() >= 2 && p.last().unwrap() == "triggered" {
                    let mut t = p[..p.len() - 1].to_vec();
                    let ev = if t.len() == 1 { Expr::Ident(t[0].clone()) } else { Expr::Hier(t.clone()) };
                    let sym = match &ev {
                        Expr::Ident(n) => self.lookup(n, sid),
                        Expr::Hier(p) => self.lookup_hier(p, sid),
                        _ => None,
                    };
                    if let Some(Sym::Sig(s)) = sym {
                        if self.sig(s).event {
                            let l = t.last_mut().unwrap();
                            *l = format!("{l}$trig");
                            let h = if t.len() == 1 { Expr::Ident(t[0].clone()) } else { Expr::Hier(t) };
                            return Some(Expr::Binary(BinOp::CaseEq, Box::new(h), Box::new(Expr::SysCall("$__ticks".into(), Vec::new()))));
                        }
                    }
                }
            }
        }
        if self.sigs.md_dims.is_empty() && self.sigs.packed_dims.is_empty() {
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
        let dims = self.sigs.md_dims.get(&s)?;
        if idx.len() < dims.len() {
            return None;
        }
        let num = |v: i64| -> Expr {
            let n = Expr::Num(ferrooxide_front::ast::dec(v.unsigned_abs()));
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
        let dims = self.sigs.packed_dims.get(&s)?;
        let b = |x: Expr| Box::new(x);
        let num = |v: i64| -> Expr {
            let n = Expr::Num(ferrooxide_front::ast::dec(v.unsigned_abs()));
            if v < 0 {
                Expr::Unary(UnOp::Neg, b(n))
            } else {
                n
            }
        };
        // an array of packed vectors (`reg [1:0][3:0] pm [0:1]`): the first index is the
        // element, the others select in its packed dimensions
        let elem;
        let (flat, idx): (&Expr, &[&Expr]) = if self.sig(s).depth > 0 {
            if self.sigs.md_dims.contains_key(&s) || idx.is_empty() {
                return None;
            }
            elem = Expr::Index(b(flat.clone()), b(idx[0].clone()));
            if idx.len() == 1 {
                return Some(elem);
            }
            (&elem, &idx[1..])
        } else {
            (flat, idx)
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
        // an element of an array of packed vectors: m[k][hi:lo] selects in its first packed
        // dimension
        let arr_elem = match base {
            Expr::Index(a, k) => match &**a {
                Expr::Ident(n) if !n.starts_with("$flat:") => match self.lookup(n, sid) {
                    Some(Sym::Sig(s)) if self.sig(s).depth > 0 && !self.sigs.md_dims.contains_key(&s) && self.sigs.packed_dims.contains_key(&s) => {
                        Some((Expr::Index(Box::new(Expr::Ident(format!("$flat:{n}"))), k.clone()), s))
                    }
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        };
        // a part of an element of the last dimension: x[i][m:l], x[i][b +: w]
        if arr_elem.is_none() {
            if let Expr::Index(..) = base {
                return self.packed_elem_part(e, base, sid);
            }
        }
        let (flat, s) = match (base, arr_elem) {
            (_, Some(x)) => x,
            (Expr::Ident(n), None) if !n.starts_with("$flat:") => match self.lookup(n, sid)? {
                Sym::Sig(s) => (Expr::Ident(format!("$flat:{n}")), s),
                _ => return None,
            },
            _ => return None,
        };
        let dims = self.sigs.packed_dims.get(&s)?;
        let ew: i64 = dims[1..].iter().map(|d| (d.0 - d.1).abs() + 1).product();
        let (l0, r0) = dims[0];
        let pos = |i: i64| if l0 >= r0 { i - r0 } else { r0 - i };
        let b = |x: Expr| Box::new(x);
        let num = |v: i64| -> Expr {
            let n = Expr::Num(ferrooxide_front::ast::dec(v.unsigned_abs()));
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
        let dims = self.sigs.packed_dims.get(&s)?;
        // (an array's first index selects the element)
        let arr = usize::from(self.sig(s).depth > 0);
        if idx.len() + 1 != dims.len() + arr {
            return None;
        }
        // the element's offset (as an expression), then the part inside it
        let Some(Expr::IndexedRange(flat, off, _, _)) = self.md_norm(inner, sid) else { return None };
        let (l, r) = dims[dims.len() - 1];
        let pos = |i: i64| if l >= r { i - r } else { r - i };
        let b = |x: Expr| Box::new(x);
        let num = |v: i64| -> Expr {
            let n = Expr::Num(ferrooxide_front::ast::dec(v.unsigned_abs()));
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
            Expr::Type(_) | Expr::Pattern(..) | Expr::NewArray(..) | Expr::Last | Expr::IncDec { .. } => false,
            Expr::Null | Expr::New(_) | Expr::NewCopy(_) | Expr::MethodCall(..) | Expr::EmptyArg => false,
            // a member of a struct parameter
            Expr::Member(..) => self.md_norm(e, sid).is_some_and(|n| self.is_const(&n, sid)),
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

    pub(crate) fn mkerr(&self, msg: String) -> ferrooxide_front::Error {
        ferrooxide_front::Error { msg, file: self.file.clone(), line: self.line }
    }

    /// Compile in a self-determined context.
    /// A value used as a condition: (slot, width); a real is true when it is not 0.0.
    pub(crate) fn truth(&mut self, e: &Expr, sid: usize) -> R<(u32, u32)> {
        if self.type_of(e, sid)?.is_real() {
            let r = self.rexpr(e, sid)?;
            let z = self.const_slot(&Const::zero(64));
            let d = self.tmp(1);
            self.emit(Instr::RCmp { op: CmpOp::Ne, d, a: r, b: z });
            return Ok((d, 1));
        }
        let (v, w, _) = self.self_expr(e, sid)?;
        Ok((v, w))
    }

    /// `++t` / `t--` in an expression: the assignment `t = t +/- 1` (as a statement would do
    /// it), then the new value (prefix) or the old one, in a temporary (64-bit f64 if `real`).
    fn inc_dec(&mut self, prefix: bool, inc: bool, t: &Expr, sid: usize, real: bool) -> R<u32> {
        let read = |c: &mut Self| -> R<u32> {
            let (v, w) = if real { (c.rexpr(t, sid)?, 64) } else { (c.self_expr(t, sid)?.0, c.type_of(t, sid)?.width()) };
            let d = c.tmp(w);
            c.emit(Instr::Copy { d, s: v, w });
            Ok(d)
        };
        let old = if prefix { None } else { Some(read(self)?) };
        let op = if inc { BinOp::Add } else { BinOp::Sub };
        let one = Expr::Num(ferrooxide_front::ast::dec(1));
        let line = self.line;
        let st = Stmt::Assign { lhs: t.clone(), rhs: Expr::Binary(op, Box::new(t.clone()), Box::new(one)), nonblocking: false, timing: None, line };
        self.stmt(&st, sid)?;
        match old {
            Some(d) => Ok(d),
            None => read(self),
        }
    }

    pub(crate) fn self_expr(&mut self, e: &Expr, sid: usize) -> R<(u32, u32, bool)> {
        let (w, s) = self.type_of(e, sid)?.ws();
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
        // a property of an object, a method's result, null
        if !self.classes.list.is_empty() || matches!(e, Expr::Null | Expr::New(_)) {
            if let Some((v, t)) = self.obj_value(e, sid)? {
                return self.dyn_to_vec(v, t, w, s);
            }
        }
        // a string member of a struct where a vector is needed: its characters
        if !self.code.str_raw && self.member_type(e, sid) == Some(Type::Str) {
            return self.str_to_vec(e, sid, w);
        }
        if let Some(n) = self.md_norm(e, sid) {
            return self.expr(&n, sid, w, s);
        }
        let ty = self.type_of(e, sid)?;
        if !self.code.str_ids.is_empty() || matches!(e, Expr::SysCall(n, _) if n == "$sformatf" || n == "$psprintf" || n == "$__tostring") {
            // a string where a vector is needed: its characters
            if !self.code.str_raw && ty.is_str() {
                return self.str_to_vec(e, sid, w);
            }
            // s[i] of a string: its character; s.len() and co.
            if let Expr::Index(a, i) = e {
                if self.is_string_scalar(a, sid) {
                    let h = self.str_expr(a, sid)?;
                    let iv = self.expr(i, sid, 32, true)?;
                    let d = self.str_op(ferrooxide_rt::StrOp::Getc, h, iv, NONE, 0);
                    return Ok(self.resize(d, 8, w, false));
                }
            }
            if let Expr::Call(name, args) = e {
                if let Some((base, m)) = self.str_method(name, sid) {
                    if let Some((mw, ms)) = Self::str_method_width(m) {
                        let d = self.str_method_call(&base, m, args, sid)?;
                        return Ok(self.resize(d, mw, w, s && ms));
                    }
                }
            }
        }
        // a real expression where an integer is needed: round
        if ty.is_real() {
            let v = self.rexpr(e, sid)?;
            let d = self.tmp(w);
            self.emit(Instr::RToI { d, s: v, w, trunc: false });
            return Ok(d);
        }
        // an element or a method call of a queue or dynamic array
        if self.sigs.has_dyn {
            if let Some((v, t)) = self.dyn_value(e, sid)? {
                return self.dyn_to_vec(v, t, w, s);
            }
        }
        // constant folding
        if !self.code.pb.last().map(|p| p.proc == NONE).unwrap_or(true) && self.is_const(e, sid) {
            if !matches!(e, Expr::Num(_) | Expr::Ident(_)) {
                // evaluate in the context width and signedness (operands are extended first)
                self.code.pb.push(PState { proc: NONE, ..Default::default() });
                let r = self.expr(e, sid, w, s);
                let st = self.code.pb.pop().unwrap();
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
            Expr::Member(..) => self.member_error(e, sid),
            Expr::Pattern(..) => {
                let r = self.typed_pattern(e, sid)?;
                self.expr(&r, sid, w, s)
            }
            Expr::NewArray(..) | Expr::Last => self.type_of(e, sid).map(|_| NONE),
            Expr::IncDec { prefix, inc, e: t } => {
                let (tw, ts) = self.type_of(t, sid)?.ws();
                let r = self.inc_dec(*prefix, *inc, t, sid, false)?;
                Ok(self.resize(r, tw, w, s && ts))
            }
            Expr::Null | Expr::New(_) | Expr::NewCopy(_) | Expr::MethodCall(..) => self.err("no classes are declared"),
            Expr::EmptyArg => self.err("an empty argument is allowed only in a call of a function or task"),
            Expr::Num(n) => {
                let c = Const::number_in(n, w, s).or_else(|m| self.err(m))?;
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
                let Some(slot) = slot else {
                    return self.err(format!("void function '{name}' has no value"));
                };
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
            B::LAnd | B::LOr if has_call(b) => {
                // short-circuit (IEEE 1800 11.4.7): the right side runs only when the left
                // does not decide the result (it is unknown, or 1 for &&, 0 for ||)
                let (av, aw) = self.truth(a, sid)?;
                let d = self.tmp(1);
                self.emit(Instr::Red { op: RedOp::Bool, d, s: av, w: aw });
                let decided = if op == B::LAnd {
                    let nz = self.tmp(1);
                    self.emit(Instr::Red { op: RedOp::Nor, d: nz, s: d, w: 1 });
                    nz
                } else {
                    d
                };
                let to_b = self.emit(Instr::Jz { c: decided, w: 1, to: 0 });
                let end = self.emit(Instr::Jmp { to: 0 });
                let h = self.here();
                self.patch(to_b, h);
                let (bv, bw) = self.truth(b, sid)?;
                let tb = self.tmp(1);
                self.emit(Instr::Red { op: RedOp::Bool, d: tb, s: bv, w: bw });
                let rop = if op == B::LAnd { RB::And } else { RB::Or };
                self.emit(Instr::Bin { op: rop, d, a: d, b: tb, w: 1 });
                let h = self.here();
                self.patch(end, h);
                Ok(self.resize(d, 1, w, false))
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
                let (ta, tb) = (self.type_of(a, sid)?, self.type_of(b, sid)?);
                if let (Type::Dyn { elem, .. }, _) | (_, Type::Dyn { elem, .. }) = (ta, tb) {
                    if !matches!(op, B::Eq | B::Ne | B::CaseEq | B::CaseNe) {
                        return self.err("queues and dynamic arrays can only be compared with == and !=");
                    }
                    let d = self.dyn_compare(matches!(op, B::Ne | B::CaseNe), a, b, elem, sid)?;
                    return Ok(self.resize(d, 1, w, false));
                }
                if ta.is_str() || tb.is_str() {
                    let cop = match op {
                        B::Eq | B::CaseEq | B::WildEq => CmpOp::Eq,
                        B::Ne | B::CaseNe | B::WildNe => CmpOp::Ne,
                        B::Lt => CmpOp::Lt,
                        B::Le => CmpOp::Le,
                        B::Gt => CmpOp::Gt,
                        _ => CmpOp::Ge,
                    };
                    let d = self.str_compare(cop, a, b, sid)?;
                    return Ok(self.resize(d, 1, w, false));
                }
                if ta.is_real() || tb.is_real() {
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
                let (wa, sa) = ta.ws();
                let (wb, sb) = tb.ws();
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
                let vw = self.type_of(&args[0], sid)?.width();
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
                    let t = self.type_of(v, sid)?;
                    let (real, w) = (t.is_real(), t.width());
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
            "$time" | "$stime" | "$__ticks" => {
                let d = self.tmp(64);
                // $__ticks: the simulation time in ticks of the precision
                let e = if name == "$__ticks" { 0 } else { self.texp(sid) };
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
                let (cw, _) = self.type_of(&Expr::SysCall(name.into(), args.to_vec()), sid)?.ws();
                let (ew, es) = match self.type_of(&args[2], sid)? {
                    Type::Real => (cw, true),
                    t => t.ws(),
                };
                let ctx = cw.max(ew);
                let v = self.expr(&args[2], sid, ctx, es)?;
                let v = self.resize(v, ctx, cw, false);
                Ok(self.resize(v, cw, w, s))
            }
            "$countones" | "$onehot" | "$onehot0" => {
                let Some(a) = args.first() else { return self.err(format!("{name} needs an argument")) };
                let (v, vw, _) = self.self_expr(a, sid)?;
                let (mode, ow) = match name { "$countones" => (0, 32), "$onehot" => (1, 1), _ => (2, 1) };
                let d = self.tmp(ow);
                self.emit(Instr::Ones { d, s: v, w: vw, mode });
                Ok(self.resize(d, ow, w, false))
            }
            // array queries of what the front end could not answer (a parameter, an
            // expression): one packed dimension, the parameter's declared range or [w-1:0]
            "$left" | "$right" | "$low" | "$high" | "$size" | "$increment" | "$dimensions" | "$unpacked_dimensions" => {
                let Some(a) = args.first() else { return self.err(format!("{name} needs an argument")) };
                if let Some(d) = args.get(1) {
                    if self.const_int(d, sid)? != 1 {
                        return self.err(format!("{name}: the dimension is out of range"));
                    }
                }
                // a queue or dynamic array: [0:size-1] at run time
                if let Type::Dyn { .. } = self.type_of(a, sid)? {
                    let size = match a {
                        Expr::Ident(n) => Expr::Call(format!("{n}.size"), vec![]),
                        Expr::Hier(p) => Expr::Call(format!("{}.size", p.join(".")), vec![]),
                        _ => return self.err(format!("{name} of this array is not supported")),
                    };
                    let last = Expr::Binary(BinOp::Sub, Box::new(size.clone()), Box::new(Expr::Num(ferrooxide_front::ast::dec(1))));
                    let zero = Expr::Num(ferrooxide_front::ast::dec(0));
                    let e = match name {
                        "$size" => size,
                        "$left" | "$low" => zero,
                        "$right" | "$high" => last,
                        "$increment" => Expr::Unary(ferrooxide_front::ast::UnOp::Neg, Box::new(Expr::Num(ferrooxide_front::ast::dec(1)))),
                        "$dimensions" | "$unpacked_dimensions" => {
                            return self.err(format!("{name} of a queue or dynamic array is not supported"));
                        }
                        _ => unreachable!(),
                    };
                    return self.expr(&e, sid, w, s);
                }
                let range = match a {
                    Expr::Ident(n) => self.param_range(n, sid),
                    _ => None,
                };
                let (l, r) = match range {
                    Some(lr) => lr,
                    None => (self.type_of(a, sid)?.width() as i64 - 1, 0),
                };
                let v = match name {
                    "$left" => l,
                    "$right" => r,
                    "$low" => l.min(r),
                    "$high" => l.max(r),
                    "$size" => (l - r).abs() + 1,
                    "$increment" => if l >= r { 1 } else { -1 },
                    "$dimensions" => 1,
                    _ => 0,
                };
                Ok(self.const_slot(&Const::from_i64(v, w, s)))
            }
            "$countbits" => {
                if args.len() < 2 {
                    return self.err("$countbits needs an expression and at least one control bit");
                }
                let (v, vw, _) = self.self_expr(&args[0], sid)?;
                let mut ctl = Vec::new();
                for c in &args[1..] {
                    ctl.push(self.self_expr(c, sid)?.0);
                }
                let d = self.tmp(32);
                self.emit(Instr::CountBits { d, s: v, w: vw, ctl: ctl.into_boxed_slice() });
                Ok(self.resize(d, 32, w, false))
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
                let (bw, _) = self.type_of(&args[0], sid)?.ws();
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
                    // no mode: a multichannel descriptor
                    None => String::new(),
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
                let t = self.type_of(var, sid)?;
                let (real, vw) = (t.is_real(), t.width());
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
            // (internal: a string's handle, for a string member in an assignment pattern)
            "$__strbits" => {
                let h = self.str_expr(&args[0], sid)?;
                Ok(self.resize(h, 64, w, false))
            }
            _ => self.err(format!("unsupported system function '{name}'")),
        }
    }

    // ------------------------------------------------------------------ functions & tasks
    /// Storage for a function/task: shared (static), or per call site for `automatic` ones
    /// (`key` distinguishes the instances).
    pub(crate) fn func_instance(&mut self, f: &'a SubroutineDecl, def_scope: usize, key: &str) -> R<()> {
        let key = (def_scope, key.to_string());
        if self.subr.func_inst.contains_key(&key) {
            return Ok(());
        }
        let mem_lo = self.prog.mem.len() as u32;
        let path = format!("{}.{}", self.scopes[def_scope].name, f.name);
        let file = self.file.clone();
        let fs = self.new_scope(path.clone(), Some(def_scope), file);
        // a method: `this`, the object's handle
        let this = match self.classes.of_scope.get(&def_scope) {
            Some(&c) => {
                let t = self.new_signal(format!("{path}.this"), 64, false, 0, 63, 0, 0, SigKind::Reg);
                self.sigs.types[t as usize] = Type::Class(c);
                self.scopes[fs].syms.insert("this".into(), Sym::Sig(t));
                Some(t)
            }
            None => None,
        };
        let ret = if f.is_task || f.ret_kind == NetKind::Int(0) {
            None
        } else if let Type::Dyn { kind, elem } = self.func_ret_type(f, def_scope)? {
            // a function returning a queue, dynamic or associative array
            Some(self.new_dyn_kind(path.clone(), &f.name, elem, kind, fs))
        } else {
            let (w, s) = self.func_ret_type(f, def_scope)?.ws();
            let (m, l) = match f.ret_kind {
                NetKind::Integer => (31, 0),
                NetKind::Real | NetKind::String | NetKind::Class => (63, 0),
                _ => self.range_vals(&f.range, def_scope)?,
            };
            let layout = self.layout_for(f.ret_ty.as_ref(), fs)?;
            let sg = self.new_signal(path.clone(), w, s, 0, m, l, 0, SigKind::Reg);
            if let t @ Type::Class(_) = self.func_ret_type(f, def_scope)? {
                self.sigs.types[sg as usize] = t;
            }
            if f.ret_kind == NetKind::Real {
                self.mark_real(sg);
            }
            if f.ret_kind == NetKind::String {
                self.mark_string(sg);
            }
            self.scopes[fs].syms.insert(f.name.clone(), Sym::Sig(sg));
            self.set_layout(sg, &f.name, layout, fs)?;
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
                NetKind::Time | NetKind::Real | NetKind::String | NetKind::Class => (m, l) = (63, 0),
                _ => {}
            }
            // packed dimensions: one flat vector [W-1:0], the dimensions kept for selects
            let mut pk: Option<Vec<(i64, i64)>> = None;
            if !d.packed.is_empty() {
                let mut dims = vec![(m, l)];
                for (a, b) in &d.packed {
                    dims.push((self.const_int(a, fs)?, self.const_int(b, fs)?));
                }
                let w: i64 = dims.iter().map(|(a, b)| (a - b).abs() + 1).product();
                (m, l) = (w - 1, 0);
                pk = Some(dims);
            }
            let w = ((m - l).abs() + 1) as u32;
            let layout = self.layout_for(d.ty.as_ref(), fs)?;
            for dl in &d.names {
                if let Some(h) = &dl.heap {
                    // a queue or dynamic array argument or variable
                    let elem = Self::elem_of(d.kind, w, signed);
                    let sg = self.new_dyn(format!("{path}.{}", dl.name), &dl.name, elem, h, fs)?;
                    if let Some(dir) = d.dir {
                        ports.push((dir, sg));
                    }
                    continue;
                }
                let (depth, lo) = if let Some((a, b)) = dl.dims.first() {
                    let (a, b) = (self.const_int(a, fs)?, self.const_int(b, fs)?);
                    (((a - b).abs() + 1) as u32, a.min(b))
                } else {
                    (0, 0)
                };
                let sg = self.new_signal(format!("{path}.{}", dl.name), w, signed, depth, m, l, lo, SigKind::Reg);
                if let Some(d) = &pk {
                    self.sigs.packed_dims.insert(sg, d.clone());
                    self.scopes[fs].syms.insert(format!("$flat:{}", dl.name), Sym::Sig(sg));
                }
                if d.kind == NetKind::Real {
                    self.mark_real(sg);
                }
                if d.kind == NetKind::String {
                    self.mark_string(sg);
                }
                if d.kind == NetKind::Event {
                    self.zero_init(sg);
                }
                if d.kind == NetKind::Class {
                    let c = self.class_of_decl(d, fs)?;
                    self.sigs.types[sg as usize] = Type::Class(c);
                    self.zero_init(sg);
                }
                self.mark_two_state(sg, d.kind);
                self.scopes[fs].syms.insert(dl.name.clone(), Sym::Sig(sg));
                self.set_layout(sg, &dl.name, layout.clone(), fs)?;
                if let Some(dir) = d.dir {
                    ports.push((dir, sg));
                } else if let (Some(init), false) = (&dl.init, f.automatic || key.1.ends_with("$const")) {
                    // a static variable: its initial value once, as for a module variable
                    // (automatic ones get it on each call, `auto_inits`)
                    let real = d.kind == NetKind::Real;
                    self.decl_init(init, sg, SigKind::Reg, d.kind, real, depth, w, None, None, &dl.name, fs)?;
                }
            }
        }
        if key.1 == f.name {
            // the static instance: reachable by hierarchical names (task.var)
            self.scopes[def_scope].children.entry(f.name.clone()).or_insert(fs);
        }
        let mem = (mem_lo, self.prog.mem.len() as u32);
        self.subr.func_inst.insert(key, FuncInst { scope: fs, ret, ports, mem, this });
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
        self.call_resolved(f, fs_def, name, None, args, sid)
    }

    /// Inline a call of `f` (declared in scope `fs_def`); `this`: the slot of the object's
    /// handle for a method.
    pub(crate) fn call_resolved(
        &mut self,
        f: &'a SubroutineDecl,
        fs_def: usize,
        name: &str,
        this: Option<u32>,
        args: &[Expr],
        sid: usize,
    ) -> R<(Option<u32>, u32, bool)> {
        // missing trailing arguments and empty ones take their default values (IEEE 1800
        // 13.5.3)
        let filled: Vec<Expr>;
        let args: &[Expr] = {
            let defaults: Vec<Option<&Expr>> =
                f.decls.iter().filter(|d| d.dir.is_some()).flat_map(|d| d.names.iter().map(|n| n.init.as_ref())).collect();
            // (`f()` of a function with one argument: no argument)
            let args = if let [Expr::EmptyArg] = args { &[][..] } else { args };
            if args.len() < defaults.len() || args.iter().any(|a| matches!(a, Expr::EmptyArg)) {
                let mut v = Vec::new();
                for k in 0..defaults.len().max(args.len()) {
                    match (args.get(k), defaults.get(k)) {
                        (Some(Expr::EmptyArg) | None, Some(Some(d))) => v.push((*d).clone()),
                        (Some(Expr::EmptyArg) | None, _) => {
                            let missing = f.decls.iter().filter(|d| d.dir.is_some()).flat_map(|d| d.names.iter()).nth(k).map(|n| n.name.clone());
                            return self.err(match missing {
                                Some(m) => format!("argument '{m}' of '{name}' has no default value"),
                                None => format!("too many arguments to '{name}'"),
                            });
                        }
                        (Some(a), _) => v.push(a.clone()),
                    }
                }
                filled = v;
                &filled
            } else {
                args
            }
        };
        // a call of a function that is being inlined: recursion, through a subroutine
        if let Some(k) = self.subr.inline_stack.iter().rev().find(|x| x.0 == fs_def && x.1 == f.name).map(|x| x.2.clone()) {
            return self.recursive_call(f, k, this, args, sid);
        }
        if self.subr.call_depth > 32 {
            return self.err(format!("calls nested too deeply in '{name}'"));
        }
        // a constant function call (elaboration): every call starts afresh, also for a static
        // function, and leaves its run-time variables alone
        let fresh = f.automatic || self.code.pb.last().is_none_or(|p| p.proc == NONE);
        let kname = if f.automatic {
            self.subr.auto_count += 1;
            format!("{}#{}", f.name, self.subr.auto_count)
        } else if fresh {
            format!("{}$const", f.name)
        } else {
            f.name.clone()
        };
        self.func_instance(f, fs_def, &kname)?;
        let key = (fs_def, kname);
        let (fscope, ret, ports, this_sig) = {
            let fi = &self.subr.func_inst[&key];
            (fi.scope, fi.ret, fi.ports.clone(), fi.this)
        };
        if args.len() > ports.len() {
            return self.err(format!("too many arguments to '{name}'"));
        }
        // an automatic function's queue result starts empty on each call
        if let (true, Some(r)) = (f.automatic, ret) {
            if let Type::Dyn { .. } = self.sig_type(r) {
                let z = self.const_slot(&Const::zero(64));
                self.emit(Instr::Store { t: Target::Whole { sig: r }, s: z, mode: rt::StoreMode::Blocking });
            }
        }
        // inputs
        for (i, a) in args.iter().enumerate() {
            let (dir, ps) = ports[i];
            if dir == Dir::Input || dir == Dir::Inout {
                let pw = self.sig(ps).width;
                let real = self.sig_type(ps);
                let pl = self.layout(ps);
                let v = self.value_for(a, sid, pw, real, pl.as_deref())?;
                self.emit(Instr::Store { t: Target::Whole { sig: ps }, s: v, mode: rt::StoreMode::Blocking });
            }
        }
        if let (Some(h), Some(t)) = (this, this_sig) {
            let ts = self.sig(t).slot;
            self.emit(Instr::Copy { d: ts, s: h, w: 64 });
            // a constructor: the base class's constructor and the property initializers
            // first, unless the body starts with `super.new(..)`
            if f.name == "new" && !crate::Compiler::calls_super_new(f) {
                let c = self.classes.of_scope[&fs_def];
                self.ctor_prologue(c, ts, None, fscope)?;
            }
        }
        self.subr.call_depth += 1;
        self.subr.inline_stack.push((fs_def, f.name.clone(), key.clone()));
        self.code.pb.last_mut().unwrap().rets.push((ret, Vec::new()));
        let saved_file = self.file.clone();
        // the body is a block `disable <task>` can end
        let bpath = format!("{}.{}", self.scopes[fs_def].name, f.name);
        let reg = self.open_block(bpath);
        let r = self.auto_inits(f, fscope, fresh).and_then(|_| self.stmt(&f.body, fscope));
        self.close_block(reg);
        self.file = saved_file;
        let (_, patches) = self.code.pb.last_mut().unwrap().rets.pop().unwrap();
        self.subr.call_depth -= 1;
        self.subr.inline_stack.pop();
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

    /// The initializers of an automatic subroutine's variables (`fresh`: also of a static
    /// one called in a constant expression), applied on each call.
    fn auto_inits(&mut self, f: &'a SubroutineDecl, fscope: usize, fresh: bool) -> R<()> {
        if !fresh {
            return Ok(());
        }
        for d in f.decls.iter().filter(|d| d.dir.is_none()) {
            for n in &d.names {
                if let Some(init) = &n.init {
                    let st = Stmt::Assign { lhs: Expr::Ident(n.name.clone()), rhs: init.clone(), nonblocking: false, timing: None, line: n.line };
                    self.stmt(&st, fscope)?;
                }
            }
        }
        Ok(())
    }

    /// A recursive call: save the function's variables (and temporaries), pass the arguments,
    /// jump to the function's subroutine; results come back in fresh temporaries.
    fn recursive_call(
        &mut self,
        f: &'a SubroutineDecl,
        key: (usize, String),
        this: Option<u32>,
        args: &[Expr],
        sid: usize,
    ) -> R<(Option<u32>, u32, bool)> {
        let (ret, ports, this_sig) = {
            let fi = &self.subr.func_inst[&key];
            (fi.ret, fi.ports.clone(), fi.this)
        };
        if args.len() > ports.len() {
            return self.err(format!("too many arguments to '{}'", f.name));
        }
        // argument values first (they may read the current frame)
        let mut vals = Vec::new();
        if let (Some(h), Some(t)) = (this, this_sig) {
            let k = self.tmp(64);
            self.emit(Instr::Copy { d: k, s: h, w: 64 });
            vals.push((t, k));
        }
        for (i, a) in args.iter().enumerate() {
            let (dir, ps) = ports[i];
            if dir == Dir::Input || dir == Dir::Inout {
                let pw = self.sig(ps).width;
                let real = self.sig_type(ps);
                let pl = self.layout(ps);
                let v = self.value_for(a, sid, pw, real, pl.as_deref())?;
                let k = self.tmp(pw);
                self.emit(Instr::Copy { d: k, s: v, w: pw });
                vals.push((ps, k));
            }
        }
        let level = self.code.pb.len();
        // recursion from another process (a fork branch inside the function) would need frames
        // shared between processes
        if self.subr.subs.iter().any(|r| r.key == key && r.level != level) {
            return self.err(format!("recursive call of '{}' from a fork branch is not supported", f.name));
        }
        let si = match self.subr.subs.iter().position(|r| r.level == level && r.key == key) {
            Some(i) => i,
            None => {
                self.subr.subs.push(SubReq { level, f, key: key.clone(), calls: Vec::new(), saves: Vec::new(), entry: None, alloc: (0, 0) });
                self.subr.subs.len() - 1
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
        self.subr.subs[si].saves.push(save);
        self.subr.subs[si].calls.push(call);
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
        let level = self.code.pb.len();
        if !self.subr.subs.iter().any(|r| r.level == level) {
            return Ok(());
        }
        // the process's own code ends before the subroutines
        self.emit(Instr::Halt);
        while let Some(i) = self.subr.subs.iter().position(|r| r.level == level && r.entry.is_none()) {
            let (f, key) = (self.subr.subs[i].f, self.subr.subs[i].key.clone());
            let (fscope, ret, ports, _) = {
                let fi = &self.subr.func_inst[&key];
                (fi.scope, fi.ret, fi.ports.clone(), fi.mem)
            };
            let entry = self.here();
            self.subr.subs[i].entry = Some(entry as usize);
            let a0 = self.prog.mem.len() as u32;
            self.subr.call_depth += 1;
            self.subr.inline_stack.push((key.0, f.name.clone(), key.clone()));
            self.code.pb.last_mut().unwrap().rets.push((ret, Vec::new()));
            let bpath = format!("{}.{}", self.scopes[key.0].name, f.name);
            let reg = self.open_block(bpath);
            self.auto_inits(f, fscope, f.automatic)?;
            let r = self.stmt(&f.body, fscope);
            // (on an error the compilation stops: the state is not cleaned up)
            r?;
            self.close_block(reg);
            let (_, patches) = self.code.pb.last_mut().unwrap().rets.pop().unwrap();
            self.subr.inline_stack.pop();
            self.subr.call_depth -= 1;
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
            self.subr.subs[i].alloc = (a0, self.prog.mem.len() as u32);
        }
        // complete calls and frames: the function's variables, what its body allocated, and the
        // process's temporaries
        let hw = self.code.scratch_top;
        let reqs: Vec<usize> = (0..self.subr.subs.len()).filter(|&i| self.subr.subs[i].level == level).collect();
        for i in reqs.iter().rev() {
            let r = self.subr.subs.remove(*i);
            let fm = self.subr.func_inst[&r.key].mem;
            let mut ranges = vec![(fm.0, fm.1 - fm.0), (r.alloc.0, r.alloc.1 - r.alloc.0)];
            if hw > 0 {
                ranges.push((0, hw));
            }
            ranges.retain(|x| x.1 > 0);
            let code = &mut self.code.pb.last_mut().unwrap().code;
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
            Expr::Type(_) | Expr::Last | Expr::Null | Expr::EmptyArg => {}
            Expr::IncDec { e, .. } => self.reads_expr(e, sid, out),
            Expr::New(v) => v.iter().for_each(|x| self.reads_expr(x, sid, out)),
            Expr::NewCopy(x) => self.reads_expr(x, sid, out),
            Expr::MethodCall(o, _, args) => {
                self.reads_expr(o, sid, out);
                args.iter().for_each(|x| self.reads_expr(x, sid, out));
            }
            Expr::NewArray(n, init) => {
                self.reads_expr(n, sid, out);
                if let Some(i) = init {
                    self.reads_expr(i, sid, out);
                }
            }
            Expr::Member(a, _) => self.reads_expr(a, sid, out),
            Expr::Pattern(_, items) => {
                for it in items {
                    let (PatItem::Pos(v) | PatItem::Key(_, v) | PatItem::Default(v)) = it;
                    self.reads_expr(v, sid, out);
                }
            }
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
                // a method of a queue reads the queue
                if let Expr::Call(name, _) = e {
                    if let Some((_, s, _, _)) = self.dyn_method(name, sid) {
                        out.push((s, NONE));
                    }
                }
                // default values of missing arguments are evaluated at the call
                if let Expr::Call(name, args) = e {
                    if let Some((f, _)) = self.lookup_func(name, sid) {
                        let defaults: Vec<&Expr> = f
                            .decls
                            .iter()
                            .filter(|d| d.dir.is_some())
                            .flat_map(|d| d.names.iter())
                            .skip(args.len())
                            .filter_map(|n| n.init.as_ref())
                            .collect();
                        for d in defaults {
                            self.reads_expr(d, sid, out);
                        }
                    }
                }
                // signals a function reads besides its arguments: they do not count for
                // sensitivity (Icarus's continuous assignments and @* follow the arguments
                // only), but the lint check (synthesis view) includes them
                if let (Expr::Call(name, _), true) = (e, self.subr.func_reads.get()) {
                    if let Some((f, fs)) = self.lookup_func(name, sid) {
                        // (functions calling each other: each is followed once)
                        let key = (fs, f.name.clone());
                        if !self.subr.func_reads_seen.borrow().contains(&key) {
                            self.subr.func_reads_seen.borrow_mut().push(key.clone());
                            let mut inner = Vec::new();
                            if let Some(fi) = self.subr.func_inst.get(&key) {
                                self.reads_stmt(&f.body, fi.scope, &mut inner);
                                let own: Vec<u32> = fi.ports.iter().map(|p| p.1).chain(fi.ret).collect();
                                out.extend(inner.into_iter().filter(|s| !own.contains(&s.0)));
                            } else {
                                // an automatic function: its own variables do not resolve in its
                                // module, the module's signals it reads do
                                self.reads_stmt(&f.body, fs, &mut inner);
                                out.extend(inner);
                            }
                            self.subr.func_reads_seen.borrow_mut().pop();
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
        if let Some(n) = self.lv_norm(e, sid) {
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
    /// Compile an expression as a real value: slot holding f64 bits (unknown word 0).
    pub(crate) fn rexpr(&mut self, e: &Expr, sid: usize) -> R<u32> {
        if let Some(n) = self.md_norm(e, sid) {
            return self.rexpr(&n, sid);
        }
        if let Expr::IncDec { prefix, inc, e: t } = e {
            if self.type_of(t, sid)?.is_real() {
                return self.inc_dec(*prefix, *inc, t, sid, true);
            }
        }
        if !self.type_of(e, sid)?.is_real() {
            let (s, w, signed) = self.self_expr(e, sid)?;
            let d = self.tmp(64);
            self.emit(Instr::IToR { d, s, w, signed });
            return Ok(d);
        }
        // a real element of a queue or dynamic array, a real property of an object
        if self.sigs.has_dyn {
            if let Some((v, _)) = self.dyn_value(e, sid)? {
                return Ok(v);
            }
        }
        if !self.classes.list.is_empty() {
            if let Some((v, _)) = self.obj_value(e, sid)? {
                return Ok(v);
            }
        }
        let rconst = |c: &mut Self, r: f64| c.const_slot(&Const { w: 64, signed: false, a: vec![r.to_bits()], b: vec![0] });
        if let Expr::Call(name, args) = e {
            if let Some((base, m)) = self.str_method(name, sid) {
                return self.str_method_call(&base, m, args, sid);
            }
        }
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
        self.code.pb.push(PState { proc: NONE, ..Default::default() });
        let r = self.rexpr(e, sid);
        let st = self.code.pb.pop().unwrap();
        let slot = r?;
        if !st.code.is_empty() {
            if let Err(m) = rt::vm::eval_const(&st.code, &mut self.prog.mem, &self.prog.signals) {
                return self.err(m);
            }
        }
        Ok(f64::from_bits(self.prog.mem[slot as usize]))
    }

    /// The value for an assignment to a target of type `ty`, `lw` bits wide, with layout
    /// `layout` (for an assignment pattern).
    pub(crate) fn value_for(&mut self, rhs: &Expr, sid: usize, lw: u32, ty: Type, layout: Option<&Packed>) -> R<u32> {
        let r = self.with_target(rhs, layout, sid)?;
        let rhs = r.as_ref().unwrap_or(rhs);
        match ty {
            Type::Real => self.rexpr(rhs, sid),
            Type::Str => self.str_expr(rhs, sid),
            Type::Dyn { kind, elem } => self.dyn_owned(rhs, kind, elem, sid),
            Type::Class(c) => self.handle_for(rhs, c, sid),
            Type::Vec { .. } => self.rhs_value(rhs, sid, lw),
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
        if self.type_of(d, sid)?.is_real() {
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
        if self.type_of(e, sid)?.is_str() {
            return Ok(rt::FileName::SVar { s: self.str_expr(e, sid)? });
        }
        let (s, w, _) = self.self_expr(e, sid)?;
        Ok(rt::FileName::Val { s, w })
    }
}
