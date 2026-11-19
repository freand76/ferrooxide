//! SystemVerilog `string`: a string value is a 64-bit handle of an interned text
//! (`Program::strings` at compile time, the run-time table in the simulator). Equal texts have
//! equal handles, so stores, change events and `==` of two strings work on the handles;
//! everything else is an `Instr::Str` operation.

use crate::expr::Base;
use crate::types::{Type, STR_METHODS};
use crate::*;
use ferrooxide_rt::{CmpOp, StrOp};

impl<'a> Compiler<'a> {
    /// The handle of a text known at compile time.
    pub(crate) fn intern(&mut self, b: &[u8]) -> u64 {
        if self.prog.strings.is_empty() {
            self.prog.strings.push(Vec::new());
        }
        if let Some(&h) = self.code.str_ids.get(b) {
            return h;
        }
        let h = self.prog.strings.len() as u64;
        self.prog.strings.push(b.to_vec());
        self.code.str_ids.insert(b.to_vec(), h);
        h
    }

    /// Make `s` a string variable (64 bits, the empty string).
    pub(crate) fn mark_string(&mut self, s: u32) {
        self.intern(b"");
        let i = &mut self.prog.signals[s as usize];
        i.string = true;
        self.sigs.types[s as usize] = Type::Str;
        let i = &self.prog.signals[s as usize];
        let (slot, n) = (i.slot as usize, 2 * rt::nw(i.width) * i.depth.max(1) as usize);
        for k in 0..n {
            self.prog.mem[slot + k] = 0;
        }
    }

    /// The string variable (or array of strings) a name refers to.
    fn string_var(&self, e: &Expr, sid: usize) -> Option<u32> {
        let sym = match e {
            Expr::Ident(n) => self.lookup(n, sid),
            Expr::Hier(p) => self.lookup_hier(p, sid),
            _ => None,
        };
        match sym {
            Some(Sym::Sig(s)) if self.sig_type(s).is_str() => Some(s),
            _ => None,
        }
    }

    /// A string variable that is not an array (`s[i]` is its character).
    pub(crate) fn is_string_scalar(&self, e: &Expr, sid: usize) -> bool {
        self.string_var(e, sid).is_some_and(|s| self.sig(s).depth == 0) || matches!(e, Expr::Ident(n) if self.is_string_param(n, sid))
    }

    /// `base.method` of a call name whose base is a string variable.
    pub(crate) fn str_method<'n>(&self, name: &'n str, sid: usize) -> Option<(Expr, &'n str)> {
        let (base, m) = name.rsplit_once('.')?;
        let parts: Vec<String> = base.split('.').map(String::from).collect();
        let e = if parts.len() == 1 { Expr::Ident(parts[0].clone()) } else { Expr::Hier(parts) };
        if self.is_string_scalar(&e, sid) {
            return Some((e, m));
        }
        // `a[i].m`: an element of an array, queue or dynamic array of strings
        let (arr, idx) = base.strip_suffix(']')?.split_once('[')?;
        if arr.contains(['.', '[']) {
            return None;
        }
        let a = Expr::Ident(arr.to_string());
        let strs = self.string_var(&a, sid).is_some_and(|s| self.sig(s).depth > 0)
            || self.dyn_var(&a, sid).is_some_and(|(_, el)| el.leaf == crate::types::Leaf::Str && el.class == NONE);
        if !strs {
            return None;
        }
        let i = match idx.strip_prefix('@') {
            Some(n) => ferrooxide_front::lower::hier_index_expr(n.parse().ok()?)?,
            None => Expr::Num(ferrooxide_front::ast::Number {
                size: None,
                signed: true,
                base: 10,
                based: false,
                digits: idx.parse::<u64>().ok()?.to_string(),
                fill: false,
            }),
        };
        Some((Expr::Index(Box::new(a), Box::new(i)), m))
    }

    /// The text of a constant string operand: a literal or a `string` parameter.
    fn const_text(&self, e: &Expr, sid: usize) -> Option<Vec<u8>> {
        let c = match e {
            Expr::Str(t) => return Some(t.chars().map(|c| c as u32 as u8).filter(|&c| c != 0).collect()),
            Expr::Ident(n) if self.is_string_param(n, sid) => match self.lookup(n, sid)? {
                Sym::Const(c) => c,
                _ => return None,
            },
            _ => return None,
        };
        Some((0..c.w / 8).rev().map(|k| ((c.a[(k * 8 / 64) as usize] >> (k * 8 % 64)) & 0xff) as u8).filter(|&x| x != 0).collect())
    }

    /// A comparison of constant strings, at least one a `string` parameter (`P == "AB"`,
    /// `P < Q`): its 1-bit result (6.16.8: lexicographic). None for other expressions.
    pub(crate) fn const_str_cmp(&self, e: &Expr, sid: usize) -> Option<Const> {
        let Expr::Binary(op, a, b) = e else { return None };
        let param = |x: &Expr| matches!(x, Expr::Ident(n) if self.is_string_param(n, sid));
        if !param(a) && !param(b) {
            return None;
        }
        let (x, y) = (self.const_text(a, sid)?, self.const_text(b, sid)?);
        let r = match op {
            BinOp::Eq | BinOp::CaseEq => x == y,
            BinOp::Ne | BinOp::CaseNe => x != y,
            BinOp::Lt => x < y,
            BinOp::Le => x <= y,
            BinOp::Gt => x > y,
            BinOp::Ge => x >= y,
            _ => return None,
        };
        Some(Const::from_i64(r as i64, 1, false))
    }

    /// Compile `e` as a string: the slot of its handle.
    pub(crate) fn str_expr(&mut self, e: &Expr, sid: usize) -> R<u32> {
        if let Some(h) = self.str_member(e, sid)? {
            return Ok(h);
        }
        // a string property of an object, a method's string result
        if !self.classes.list.is_empty() {
            if let Some((v, t)) = self.obj_value(e, sid)? {
                if t != Type::Str {
                    return self.err("not a string");
                }
                return Ok(v);
            }
        }
        // a string element of a queue or dynamic array
        if self.sigs.has_dyn {
            if let Some((v, t)) = self.dyn_value(e, sid)? {
                if t != Type::Str {
                    return self.err("not a string");
                }
                return Ok(v);
            }
        }
        match e {
            Expr::Str(t) => {
                // (the lexer keeps one char per byte of the literal)
                let b: Vec<u8> = t.chars().map(|c| c as u32 as u8).filter(|&c| c != 0).collect();
                let h = self.intern(&b);
                return Ok(self.const_slot(&Const::from_i64(h as i64, 64, false)));
            }
            // a `string` parameter: its text
            Expr::Ident(n) if self.is_string_param(n, sid) => {
                if let Some(b) = self.const_text(e, sid) {
                    let h = self.intern(&b);
                    return Ok(self.const_slot(&Const::from_i64(h as i64, 64, false)));
                }
            }
            Expr::Ident(_) | Expr::Hier(_) if self.string_var(e, sid).is_some() => {
                if let Some(Base::Sig(s)) = self.resolve_base(e, sid)? {
                    return Ok(self.sig(s).slot);
                }
            }
            Expr::Index(a, _) if self.array_of(a, sid).is_some_and(|s| self.sig_type(s).is_str()) => {
                // an element of a string array: its handle
                self.code.str_raw = true;
                let r = self.expr(e, sid, 64, false);
                self.code.str_raw = false;
                return r;
            }
            Expr::Concat(v) => {
                let mut acc: Option<u32> = None;
                for x in v {
                    let h = self.str_expr(x, sid)?;
                    acc = Some(match acc {
                        None => h,
                        Some(p) => self.str_op(StrOp::Concat, p, h, NONE, 0),
                    });
                }
                return Ok(match acc {
                    Some(h) => h,
                    None => self.str_expr(&Expr::Str(String::new()), sid)?,
                });
            }
            Expr::Repl(n, v) => {
                let h = self.str_expr(&Expr::Concat(v.clone()), sid)?;
                let c = self.expr(n, sid, 32, true)?;
                return Ok(self.str_op(StrOp::Repl, h, c, NONE, 0));
            }
            Expr::Cond(c, t, f) => {
                // an unknown condition gives the empty string (as in the reference)
                let (cv, cw) = self.truth(c, sid)?;
                let d = self.tmp(64);
                let b = self.tmp(1);
                self.emit(Instr::Red { op: rt::RedOp::Bool, d: b, s: cv, w: cw });
                let xc = self.const_slot(&Const::fill('x', 1));
                let isx = self.tmp(1);
                self.emit(Instr::Cmp { op: CmpOp::CaseNe, d: isx, a: b, b: xc, w: 1 });
                let jx = self.emit(Instr::Jz { c: isx, w: 1, to: 0 });
                let jz = self.emit(Instr::Jz { c: cv, w: cw, to: 0 });
                let tv = self.str_expr(t, sid)?;
                self.emit(Instr::Copy { d, s: tv, w: 64 });
                let j = self.emit(Instr::Jmp { to: 0 });
                let h = self.here();
                self.patch(jz, h);
                let fv = self.str_expr(f, sid)?;
                self.emit(Instr::Copy { d, s: fv, w: 64 });
                let j2 = self.emit(Instr::Jmp { to: 0 });
                let h = self.here();
                self.patch(jx, h);
                // (both strings when they are equal)
                let tv = self.str_expr(t, sid)?;
                let fv = self.str_expr(f, sid)?;
                let eq = self.tmp(1);
                self.emit(Instr::Str { op: StrOp::Cmp(CmpOp::Eq), d: eq, a: tv, b: fv, c: NONE, w: 1 });
                let e0 = self.str_expr(&Expr::Str(String::new()), sid)?;
                self.emit(Instr::Mux { d, c: eq, cw: 1, t: tv, f: e0, w: 64 });
                let h = self.here();
                self.patch(j, h);
                self.patch(j2, h);
                return Ok(d);
            }
            Expr::Call(name, args) => {
                if let Some((base, m)) = self.str_method(name, sid) {
                    if STR_METHODS.contains(&m) {
                        return self.str_method_call(&base, m, args, sid);
                    }
                }
                if self.type_of(e, sid)?.is_str() {
                    // a function returning a string: its result variable holds the handle
                    self.code.str_raw = true;
                    let r = self.expr(e, sid, 64, false);
                    self.code.str_raw = false;
                    return r;
                }
            }
            // string'(e)
            Expr::SysCall(n, args) if n == "$__tostring" && args.len() == 1 => {
                return self.str_expr(&args[0], sid);
            }
            Expr::SysCall(n, args) if n == "$sformatf" || n == "$psprintf" => {
                let opt: Vec<Option<Expr>> = args.iter().cloned().map(Some).collect();
                let fa = self.fmt_args(&opt, sid)?;
                let d = self.tmp(64);
                let scope = self.scopes[sid].name.clone();
                let texp = self.texp(sid);
                self.emit(Instr::Sys(Box::new(rt::SysTask::SFormat { d, w: 0, args: fa, scope, texp, radix: 'd' })));
                return Ok(d);
            }
            _ => {}
        }
        // a vector: its characters
        let (v, w, _) = self.self_expr(e, sid)?;
        Ok(self.str_op(StrOp::FromVec, v, NONE, NONE, w))
    }

    /// Emit a string operation with a fresh destination (64 bits: a handle).
    pub(crate) fn str_op(&mut self, op: StrOp, a: u32, b: u32, c: u32, w: u32) -> u32 {
        let d = self.tmp(64);
        self.emit(Instr::Str { op, d, a, b, c, w });
        d
    }

    /// A string in a vector context: its characters in `w` bits.
    pub(crate) fn str_to_vec(&mut self, e: &Expr, sid: usize, w: u32) -> R<u32> {
        let h = self.str_expr(e, sid)?;
        let d = self.tmp(w);
        self.emit(Instr::Str { op: StrOp::ToVec, d, a: h, b: NONE, c: NONE, w });
        Ok(d)
    }

    /// A comparison with a string operand: lexicographic (1 bit).
    pub(crate) fn str_compare(&mut self, op: CmpOp, a: &Expr, b: &Expr, sid: usize) -> R<u32> {
        let (x, y) = (self.str_expr(a, sid)?, self.str_expr(b, sid)?);
        let d = self.tmp(1);
        self.emit(Instr::Str { op: StrOp::Cmp(op), d, a: x, b: y, c: NONE, w: 1 });
        Ok(d)
    }

    /// The width of a string method's result in a vector context, None for methods that give
    /// a string or a real.
    pub(crate) fn str_method_width(m: &str) -> Option<(u32, bool)> {
        Some(match m {
            "len" | "compare" | "icompare" | "atoi" | "atohex" | "atooct" | "atobin" => (32, true),
            "getc" => (8, false),
            _ => return None,
        })
    }

    /// `s.m(args)` as a value: a handle for the string methods, else a vector (`w` bits) or
    /// a real (atoreal).
    pub(crate) fn str_method_call(&mut self, base: &Expr, m: &str, args: &[Expr], sid: usize) -> R<u32> {
        let s = self.str_expr(base, sid)?;
        let arg = |me: &mut Self, k: usize| -> R<u32> {
            match args.get(k) {
                Some(e) => me.expr(e, sid, 32, true),
                None => me.err(format!("string method '{m}' needs more arguments")),
            }
        };
        Ok(match m {
            "len" => self.str_op_w(StrOp::Len, s, NONE, NONE, 32),
            "getc" => {
                let i = arg(self, 0)?;
                self.str_op_w(StrOp::Getc, s, i, NONE, 8)
            }
            "toupper" => self.str_op(StrOp::Upper, s, NONE, NONE, 0),
            "tolower" => self.str_op(StrOp::Lower, s, NONE, NONE, 0),
            "substr" => {
                let (i, j) = (arg(self, 0)?, arg(self, 1)?);
                self.str_op(StrOp::Substr, s, i, j, 0)
            }
            "compare" | "icompare" => {
                let o = match args.first() {
                    Some(e) => self.str_expr(e, sid)?,
                    None => return self.err(format!("string method '{m}' needs an argument")),
                };
                let op = if m == "compare" { StrOp::Compare } else { StrOp::Icompare };
                self.str_op_w(op, s, o, NONE, 32)
            }
            "atoi" | "atohex" | "atooct" | "atobin" => {
                let op = match m {
                    "atoi" => StrOp::Atoi,
                    "atohex" => StrOp::Atohex,
                    "atooct" => StrOp::Atooct,
                    _ => StrOp::Atobin,
                };
                self.str_op_w(op, s, NONE, NONE, 32)
            }
            "atoreal" => self.str_op_w(StrOp::Atoreal, s, NONE, NONE, 64),
            _ => return self.err(format!("unknown string method '{m}'")),
        })
    }

    /// A string operation whose result is a `w`-bit vector (or a real).
    fn str_op_w(&mut self, op: StrOp, a: u32, b: u32, c: u32, w: u32) -> u32 {
        let d = self.tmp(w);
        self.emit(Instr::Str { op, d, a, b, c, w });
        d
    }

    /// A string method called as a task: `s.putc(i, c)`, `s.itoa(v)` and co. change `s`.
    /// Returns false if `name` is no such call.
    pub(crate) fn str_task(&mut self, name: &str, args: &[Expr], sid: usize) -> R<bool> {
        let Some((base, m)) = self.str_method(name, sid) else { return Ok(false) };
        let s = self.str_expr(&base, sid)?;
        let new = match m {
            "putc" => {
                if args.len() != 2 {
                    return self.err("putc needs an index and a character");
                }
                let i = self.expr(&args[0], sid, 32, true)?;
                let c = self.expr(&args[1], sid, 8, false)?;
                self.str_op(StrOp::Putc, s, i, c, 0)
            }
            "itoa" | "hextoa" | "octtoa" | "bintoa" => {
                let Some(a) = args.first() else { return self.err(format!("{m} needs a value")) };
                let (v, w, signed) = self.self_expr(a, sid)?;
                let op = match m {
                    "itoa" => StrOp::Itoa,
                    "hextoa" => StrOp::Hextoa,
                    "octtoa" => StrOp::Octtoa,
                    _ => StrOp::Bintoa,
                };
                self.str_op(op, v, NONE, if signed { 1 } else { 0 }, w)
            }
            "realtoa" => {
                let Some(a) = args.first() else { return self.err("realtoa needs a value") };
                let v = self.rexpr(a, sid)?;
                self.str_op(StrOp::Realtoa, v, NONE, NONE, 64)
            }
            _ => return Ok(false),
        };
        if self.sigs.has_dyn && self.dyn_elem_store(&base, new, sid)? {
            return Ok(true);
        }
        let l = stmt::Lhs::Expr(&base, sid);
        self.store_value(&l, new, 64, rt::StoreMode::Blocking)?;
        Ok(true)
    }

    /// `s[i] = c` for a string `s`: putc. Returns false if `lhs` is no such target.
    pub(crate) fn str_index_assign(&mut self, lhs: &Expr, rhs: &Expr, sid: usize) -> R<bool> {
        let Expr::Index(base, i) = lhs else { return Ok(false) };
        if !self.is_string_scalar(base, sid) {
            return Ok(false);
        }
        let s = self.str_expr(base, sid)?;
        let iv = self.expr(i, sid, 32, true)?;
        let c = self.rhs_value(rhs, sid, 8)?;
        let new = self.str_op(StrOp::Putc, s, iv, c, 0);
        let l = stmt::Lhs::Expr(base, sid);
        self.store_value(&l, new, 64, rt::StoreMode::Blocking)?;
        Ok(true)
    }
}
