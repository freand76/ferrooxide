//! Queues and dynamic arrays (IEEE 1800 7.5, 7.10): a variable holds the 64-bit handle of an
//! object on the simulator's heap (0: the empty array). Each variable owns its object: an
//! assignment of another variable's array copies it, one of a new array (`new[n]`, a
//! concatenation, a slice) takes it over, and the target's old object is freed. Element
//! writes and the methods change the object in place (`Instr::Heap`) and wake the waiters on
//! the variable.

use crate::types::{Elem, Leaf, Type};
use crate::*;
use ferrooxide_rt::{HeapOp, StrOp};

/// Methods of queues and dynamic arrays.
const METHODS: &[&str] = &["size", "push_back", "push_front", "pop_back", "pop_front", "insert", "delete"];

impl<'a> Compiler<'a> {
    /// The element type of a declaration of kind `kind`, `w` bits wide.
    pub(crate) fn elem_of(kind: NetKind, w: u32, signed: bool) -> Elem {
        let leaf = match kind {
            NetKind::Real => Leaf::Real,
            NetKind::String => Leaf::Str,
            _ => Leaf::Bits,
        };
        let two_state = matches!(kind, NetKind::Bit) || matches!(kind, NetKind::Int(n) if n > 0);
        let w = if leaf == Leaf::Bits { w } else { 64 };
        Elem { w, signed: signed || leaf == Leaf::Real, leaf, two_state }
    }

    /// Declare `name` in scope `sid` as a queue (or dynamic array) of `elem` (full name
    /// `full`): a 64-bit handle, the empty array.
    pub(crate) fn new_dyn(&mut self, full: String, name: &str, elem: Elem, queue: bool, sid: usize) -> u32 {
        let s = self.new_signal(full, 64, false, 0, 63, 0, 0, SigKind::Reg);
        let slot = self.sig(s).slot as usize;
        self.prog.mem[slot] = 0;
        self.prog.mem[slot + 1] = 0;
        self.sigs.types[s as usize] = Type::Dyn { queue, elem };
        self.sigs.has_dyn = true;
        if elem.leaf == Leaf::Str {
            self.intern(b"");
        }
        self.scopes[sid].syms.insert(name.to_string(), Sym::Sig(s));
        s
    }

    fn heap_op(&mut self, op: HeapOp, d: u32, a: u32, b: u32, c: u32, elem: Elem) {
        self.emit(Instr::Heap { op, d, a, b, c, w: elem.w, x: elem.x() });
    }

    /// The queue or dynamic array variable a name refers to: (signal, element type).
    pub(crate) fn dyn_var(&self, e: &Expr, sid: usize) -> Option<(u32, Elem)> {
        let sym = match e {
            Expr::Ident(n) => self.lookup(n, sid)?,
            Expr::Hier(p) => self.lookup_hier(p, sid)?,
            _ => return None,
        };
        match sym {
            Sym::Sig(s) => match self.sig_type(s) {
                Type::Dyn { elem, .. } => Some((s, elem)),
                _ => None,
            },
            _ => None,
        }
    }

    /// `base.method` of a call name whose base is a queue or dynamic array: (base, signal,
    /// element type, method).
    pub(crate) fn dyn_method<'n>(&self, name: &'n str, sid: usize) -> Option<(Expr, u32, Elem, &'n str)> {
        let (base, m) = name.rsplit_once('.')?;
        if !METHODS.contains(&m) {
            return None;
        }
        let parts: Vec<String> = base.split('.').map(String::from).collect();
        let e = if parts.len() == 1 { Expr::Ident(parts[0].clone()) } else { Expr::Hier(parts) };
        let (s, elem) = self.dyn_var(&e, sid)?;
        Some((e, s, elem, m))
    }

    /// `q.size` without parentheses as a call.
    pub(crate) fn dyn_hier_call(&self, e: &Expr, sid: usize) -> Option<Expr> {
        let Expr::Hier(p) = e else { return None };
        let name = p.join(".");
        self.dyn_method(&name, sid)?;
        Some(Expr::Call(name, vec![]))
    }

    /// The type of an element read, a method call or a slice of a queue or dynamic array.
    pub(crate) fn dyn_type_of(&self, e: &Expr, sid: usize) -> Option<R<Type>> {
        match e {
            Expr::Index(a, _) => Some(Ok(self.dyn_var(a, sid)?.1.ty())),
            Expr::Range(a, ..) => {
                let (s, _) = self.dyn_var(a, sid)?;
                Some(Ok(self.sig_type(s)))
            }
            Expr::Call(name, _) => {
                let (_, _, elem, m) = self.dyn_method(name, sid)?;
                Some(match m {
                    "size" => Ok(Type::vec(32, true)),
                    "pop_back" | "pop_front" => Ok(elem.ty()),
                    _ => self.err(format!("'{m}' gives no value")),
                })
            }
            _ => None,
        }
    }

    /// `e` with `$` replaced by the last index of the queue `q`.
    fn subst_last(e: &Expr, q: &Expr) -> Expr {
        let b = |x: &Expr| Box::new(Self::subst_last(x, q));
        match e {
            Expr::Last => {
                let name = match q {
                    Expr::Ident(n) => n.clone(),
                    Expr::Hier(p) => p.join("."),
                    _ => unreachable!(),
                };
                let size = Expr::Call(format!("{name}.size"), vec![]);
                Expr::Binary(BinOp::Sub, Box::new(size), Box::new(Expr::Num(ferrooxide_front::ast::dec(1))))
            }
            Expr::Unary(op, a) => Expr::Unary(*op, b(a)),
            Expr::Binary(op, x, y) => Expr::Binary(*op, b(x), b(y)),
            Expr::Cond(c, x, y) => Expr::Cond(b(c), b(x), b(y)),
            e => e.clone(),
        }
    }

    /// An index of queue `q` (`$`: its last index) as a 32-bit signed value.
    fn dyn_index(&mut self, q: &Expr, i: &Expr, sid: usize) -> R<u32> {
        let i = Self::subst_last(i, q);
        self.expr(&i, sid, 32, true)
    }

    /// An element read or a method call of a queue or dynamic array: the slot of the value
    /// (an element as stored: `elem.w` bits, a real's bits, a string handle) and its type.
    pub(crate) fn dyn_value(&mut self, e: &Expr, sid: usize) -> R<Option<(u32, Type)>> {
        match e {
            Expr::Index(a, i) => {
                let Some((s, elem)) = self.dyn_var(a, sid) else { return Ok(None) };
                let ix = self.dyn_index(a, i, sid)?;
                let d = self.tmp(elem.w);
                let slot = self.sig(s).slot;
                self.heap_op(HeapOp::Get, d, slot, ix, NONE, elem);
                Ok(Some((d, elem.ty())))
            }
            Expr::Call(name, args) => {
                let Some((_, s, elem, m)) = self.dyn_method(name, sid) else { return Ok(None) };
                let slot = self.sig(s).slot;
                match m {
                    "size" => {
                        let d = self.tmp(32);
                        self.heap_op(HeapOp::Size, d, slot, NONE, NONE, elem);
                        Ok(Some((d, Type::vec(32, true))))
                    }
                    "pop_back" | "pop_front" => {
                        if !args.is_empty() {
                            return self.err(format!("{m}() takes no arguments"));
                        }
                        let d = self.tmp(elem.w);
                        let op = if m == "pop_back" { HeapOp::PopBack } else { HeapOp::PopFront };
                        self.heap_op(op, d, slot, NONE, NONE, elem);
                        self.emit(Instr::Touch { sig: s });
                        Ok(Some((d, elem.ty())))
                    }
                    _ => self.err(format!("'{m}' gives no value")),
                }
            }
            _ => Ok(None),
        }
    }

    /// An element value (`e` of the element type) for storing into an array of `elem`.
    fn elem_value(&mut self, e: &Expr, elem: Elem, sid: usize) -> R<u32> {
        self.value_for(e, sid, elem.w, elem.ty(), None)
    }

    /// A queue or dynamic array value: the slot of its handle and whether the array is new
    /// (else it belongs to a variable and is copied where it is stored).
    fn dyn_rhs(&mut self, e: &Expr, elem: Elem, sid: usize) -> R<(u32, bool)> {
        if let Some((s, el)) = self.dyn_var(e, sid) {
            if el != elem && (el.w != elem.w || el.leaf != elem.leaf) {
                return self.err("the arrays have different element types");
            }
            return Ok((self.sig(s).slot, false));
        }
        match e {
            Expr::NewArray(n, init) => {
                let size = self.expr(n, sid, 32, true)?;
                let src = match init {
                    Some(i) => self.dyn_rhs(i, elem, sid)?.0,
                    None => NONE,
                };
                let d = self.tmp(64);
                self.heap_op(HeapOp::New, d, NONE, size, src, elem);
                Ok((d, true))
            }
            Expr::Concat(_) | Expr::Pattern(None, _) => {
                let parts: Vec<&Expr> = match e {
                    Expr::Concat(v) => v.iter().collect(),
                    Expr::Pattern(_, items) => {
                        let mut v = Vec::new();
                        for it in items {
                            match it {
                                PatItem::Pos(x) => v.push(x),
                                _ => return self.err("a keyed assignment pattern for a queue or dynamic array is not supported"),
                            }
                        }
                        v
                    }
                    _ => unreachable!(),
                };
                // an empty array, then the parts appended
                let d = self.tmp(64);
                let z = self.const_slot(&Const::zero(64));
                self.emit(Instr::Copy { d, s: z, w: 64 });
                for p in parts {
                    if matches!(self.type_of(p, sid), Ok(Type::Dyn { .. })) {
                        let (h, _) = self.dyn_rhs(p, elem, sid)?;
                        self.heap_op(HeapOp::Append, NONE, d, NONE, h, elem);
                    } else {
                        let v = self.elem_value(p, elem, sid)?;
                        self.heap_op(HeapOp::PushBack, NONE, d, NONE, v, elem);
                    }
                }
                Ok((d, true))
            }
            Expr::Range(a, lo, hi) if self.dyn_var(a, sid).is_some() => {
                let (s, el) = self.dyn_var(a, sid).unwrap();
                let (lo, hi) = (self.dyn_index(a, lo, sid)?, self.dyn_index(a, hi, sid)?);
                let d = self.tmp(64);
                let slot = self.sig(s).slot;
                self.heap_op(HeapOp::Slice, d, slot, lo, hi, el);
                Ok((d, true))
            }
            Expr::Cond(c, x, y) => {
                // (both arrays are made; the condition picks one)
                let x = self.dyn_owned(x, elem, sid)?;
                let y = self.dyn_owned(y, elem, sid)?;
                let (cv, cw) = self.truth(c, sid)?;
                let b = self.tmp(1);
                self.emit(Instr::Red { op: rt::RedOp::Bool, d: b, s: cv, w: cw });
                let d = self.tmp(64);
                self.emit(Instr::Mux { d, c: b, cw: 1, t: x, f: y, w: 64 });
                Ok((d, true))
            }
            Expr::Call(name, args) if self.lookup_func(name, sid).is_some() => {
                // a function returning an array: its result variable
                match self.call(name, args, sid, false)? {
                    (Some(slot), _, _) => Ok((slot, false)),
                    _ => self.err(format!("'{name}' gives no value")),
                }
            }
            _ => self.err("expected a queue or dynamic array value"),
        }
    }

    /// A queue or dynamic array value as a new array (a variable's array copied).
    pub(crate) fn dyn_owned(&mut self, e: &Expr, elem: Elem, sid: usize) -> R<u32> {
        let (h, fresh) = self.dyn_rhs(e, elem, sid)?;
        if fresh {
            return Ok(h);
        }
        let d = self.tmp(64);
        self.heap_op(HeapOp::Copy, d, h, NONE, NONE, elem);
        Ok(d)
    }

    /// Free the array of the queue variable assigned by `lhs` (before a blocking assignment
    /// of a new array to it).
    pub(crate) fn dyn_free_target(&mut self, lhs: &Expr, sid: usize) {
        if let Some((s, elem)) = self.dyn_var(lhs, sid) {
            let slot = self.sig(s).slot;
            self.heap_op(HeapOp::Free, NONE, slot, NONE, NONE, elem);
        }
    }

    /// `q[i] = v` of a queue or dynamic array: returns false if `lhs` is no such element.
    pub(crate) fn dyn_elem_assign(&mut self, lhs: &Expr, rhs: &Expr, sid: usize) -> R<bool> {
        let Expr::Index(a, i) = lhs else { return Ok(false) };
        let Some((s, elem)) = self.dyn_var(a, sid) else { return Ok(false) };
        let v = self.elem_value(rhs, elem, sid)?;
        let ix = self.dyn_index(a, i, sid)?;
        let slot = self.sig(s).slot;
        self.heap_op(HeapOp::Set, NONE, slot, ix, v, elem);
        self.emit(Instr::Touch { sig: s });
        Ok(true)
    }

    /// `q.method(args);` of a queue or dynamic array: returns false if `name` is no such
    /// method.
    pub(crate) fn dyn_task(&mut self, name: &str, args: &[Expr], sid: usize) -> R<bool> {
        let Some((base, s, elem, m)) = self.dyn_method(name, sid) else { return Ok(false) };
        let slot = self.sig(s).slot;
        let nargs = |c: &Self, n: usize| -> R<()> {
            if args.len() != n {
                return c.err(format!("{m}() takes {n} argument{}", if n == 1 { "" } else { "s" }));
            }
            Ok(())
        };
        match m {
            "push_back" | "push_front" => {
                nargs(self, 1)?;
                let v = self.elem_value(&args[0], elem, sid)?;
                let op = if m == "push_back" { HeapOp::PushBack } else { HeapOp::PushFront };
                self.heap_op(op, NONE, slot, NONE, v, elem);
            }
            "insert" => {
                nargs(self, 2)?;
                let ix = self.dyn_index(&base, &args[0], sid)?;
                let v = self.elem_value(&args[1], elem, sid)?;
                self.heap_op(HeapOp::Insert, NONE, slot, ix, v, elem);
            }
            "delete" => {
                let ix = match args {
                    [] => NONE,
                    [i] => self.dyn_index(&base, i, sid)?,
                    _ => return self.err("delete() takes at most 1 argument"),
                };
                self.heap_op(HeapOp::Delete, NONE, slot, ix, NONE, elem);
            }
            // a value method called for its effect (pop) or ignored (size)
            _ => {
                self.dyn_value(&Expr::Call(name.to_string(), args.to_vec()), sid)?;
                return Ok(true);
            }
        }
        self.emit(Instr::Touch { sig: s });
        Ok(true)
    }

    /// `a == b` / `a != b` of queues or dynamic arrays (1 bit).
    pub(crate) fn dyn_compare(&mut self, ne: bool, a: &Expr, b: &Expr, elem: Elem, sid: usize) -> R<u32> {
        let (x, _) = self.dyn_rhs(a, elem, sid)?;
        let (y, _) = self.dyn_rhs(b, elem, sid)?;
        let d = self.tmp(1);
        self.heap_op(HeapOp::Eq, d, x, y, NONE, elem);
        if ne {
            let n = self.tmp(1);
            self.emit(Instr::Un { op: rt::UnOp::Not, d: n, s: d, w: 1 });
            return Ok(n);
        }
        Ok(d)
    }

    /// An element value of type `t` in slot `v` where a `w`-bit vector is needed.
    pub(crate) fn dyn_to_vec(&mut self, v: u32, t: Type, w: u32, s: bool) -> R<u32> {
        Ok(match t {
            Type::Vec { w: ew, signed } => self.resize(v, ew, w, s && signed),
            Type::Real => {
                let d = self.tmp(w);
                self.emit(Instr::RToI { d, s: v, w, trunc: false });
                d
            }
            Type::Str => self.str_op(StrOp::ToVec, v, NONE, NONE, w),
            Type::Dyn { .. } => return self.err("a queue or dynamic array is not a vector"),
        })
    }
}
