//! SystemVerilog `string`: a string value is a 64-bit handle of an interned text
//! (`Program::strings` at compile time, the run-time table in the simulator). Equal texts have
//! equal handles, so stores, change events and `==` of two strings work on the handles;
//! everything else is an `Instr::Str` operation.

use crate::expr::Base;
use crate::*;
use ferrooxide_rt::{CmpOp, StrOp};

/// The kind of value an assignment target holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VTy {
    Vec,
    Real,
    Str,
}

/// String methods that give a string.
const STR_METHODS: &[&str] = &["substr", "toupper", "tolower"];

impl<'a> Compiler<'a> {
    /// The handle of a text known at compile time.
    pub(crate) fn intern(&mut self, b: &[u8]) -> u64 {
        if self.prog.strings.is_empty() {
            self.prog.strings.push(Vec::new());
        }
        if let Some(&h) = self.str_ids.get(b) {
            return h;
        }
        let h = self.prog.strings.len() as u64;
        self.prog.strings.push(b.to_vec());
        self.str_ids.insert(b.to_vec(), h);
        h
    }

    /// Make `s` a string variable (64 bits, the empty string).
    pub(crate) fn mark_string(&mut self, s: u32) {
        self.intern(b"");
        let i = &mut self.prog.signals[s as usize];
        i.string = true;
        let (slot, n) = (i.slot as usize, 2 * rt::nw(i.width) * i.depth.max(1) as usize);
        for k in 0..n {
            self.prog.mem[slot + k] = 0;
        }
    }

    pub(crate) fn sig_vty(&self, s: u32) -> VTy {
        let i = self.sig(s);
        if i.string {
            VTy::Str
        } else if i.real {
            VTy::Real
        } else {
            VTy::Vec
        }
    }

    /// The kind of value an assignment target holds.
    pub(crate) fn lvalue_ty(&self, e: &Expr, sid: usize) -> VTy {
        if self.is_string_var(e, sid) {
            VTy::Str
        } else if self.lvalue_is_real(e, sid) {
            VTy::Real
        } else {
            VTy::Vec
        }
    }

    /// A string variable or an element of a string array.
    pub(crate) fn is_string_var(&self, e: &Expr, sid: usize) -> bool {
        match e {
            Expr::Ident(n) => matches!(self.lookup(n, sid), Some(Sym::Sig(s)) if self.sig(s).string),
            Expr::Hier(p) => matches!(self.lookup_hier(p, sid), Some(Sym::Sig(s)) if self.sig(s).string),
            Expr::Index(a, _) => self.array_of(a, sid).is_some_and(|s| self.sig(s).string),
            _ => false,
        }
    }

    /// A string variable that is not an array (`s[i]` is its character).
    pub(crate) fn is_string_scalar(&self, e: &Expr, sid: usize) -> bool {
        let sym = match e {
            Expr::Ident(n) => self.lookup(n, sid),
            Expr::Hier(p) => self.lookup_hier(p, sid),
            _ => None,
        };
        matches!(sym, Some(Sym::Sig(s)) if self.sig(s).string && self.sig(s).depth == 0)
    }

    /// `base.method` of a call name whose base is a string variable.
    pub(crate) fn str_method<'n>(&self, name: &'n str, sid: usize) -> Option<(Expr, &'n str)> {
        let (base, m) = name.rsplit_once('.')?;
        let parts: Vec<String> = base.split('.').map(String::from).collect();
        let e = if parts.len() == 1 { Expr::Ident(parts[0].clone()) } else { Expr::Hier(parts) };
        self.is_string_var(&e, sid).then_some((e, m))
    }

    /// An expression whose value is a string (not a literal: that is a vector unless the
    /// context makes it a string).
    pub(crate) fn is_string(&self, e: &Expr, sid: usize) -> bool {
        if let Expr::SysCall(n, _) = e {
            return n == "$sformatf" || n == "$psprintf" || n == "$__tostring";
        }
        if self.str_ids.is_empty() {
            return false;
        }
        match e {
            Expr::Ident(_) | Expr::Hier(_) => self.is_string_var(e, sid),
            Expr::Index(a, _) => self.array_of(a, sid).is_some_and(|s| self.sig(s).string),
            Expr::Concat(v) => v.iter().any(|x| self.is_string(x, sid)),
            Expr::Repl(_, v) => v.iter().any(|x| self.is_string(x, sid)),
            Expr::Cond(_, t, f) => self.is_string(t, sid) || self.is_string(f, sid),
            Expr::Call(name, _) => match self.str_method(name, sid) {
                Some((_, m)) => STR_METHODS.contains(&m),
                None => self.lookup_func(name, sid).is_some_and(|(f, _)| f.ret_kind == NetKind::String),
            },
            Expr::SysCall(n, _) => n == "$sformatf" || n == "$psprintf" || n == "$__tostring",
            _ => false,
        }
    }

    /// Compile `e` as a string: the slot of its handle.
    pub(crate) fn str_expr(&mut self, e: &Expr, sid: usize) -> R<u32> {
        match e {
            Expr::Str(t) => {
                // (the lexer keeps one char per byte of the literal)
                let b: Vec<u8> = t.chars().map(|c| c as u32 as u8).filter(|&c| c != 0).collect();
                let h = self.intern(&b);
                return Ok(self.const_slot(&Const::from_i64(h as i64, 64, false)));
            }
            Expr::Ident(_) | Expr::Hier(_) if self.is_string_var(e, sid) => {
                if let Some(Base::Sig(s)) = self.resolve_base(e, sid)? {
                    return Ok(self.sig(s).slot);
                }
            }
            Expr::Index(a, _) if self.array_of(a, sid).is_some_and(|s| self.sig(s).string) => {
                // an element of a string array: its handle
                self.str_raw = true;
                let r = self.expr(e, sid, 64, false);
                self.str_raw = false;
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
                if self.is_string(e, sid) {
                    // a function returning a string: its result variable holds the handle
                    self.str_raw = true;
                    let r = self.expr(e, sid, 64, false);
                    self.str_raw = false;
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
