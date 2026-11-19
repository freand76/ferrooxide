//! Queues, dynamic and associative arrays (IEEE 1800 7.5, 7.8, 7.10): a variable holds the
//! 64-bit handle of an object on the simulator's heap (0: the empty array). Each variable owns its object: an
//! assignment of another variable's array copies it, one of a new array (`new[n]`, a
//! concatenation, a slice) takes it over, and the target's old object is freed. Element
//! writes and the methods change the object in place (`Instr::Heap`) and wake the waiters on
//! the variable. A class property can hold an array too: its handle is read into a temporary,
//! changed there and written back to the object (`DynArr`).

use crate::types::{DynKind, Elem, Leaf, Type};
use crate::*;
use ferrooxide_rt::{AssocOp, HeapOp, KeyKind, StrOp};

/// Methods of queues, dynamic and associative arrays.
const METHODS: &[&str] = &[
    "size", "push_back", "push_front", "pop_back", "pop_front", "insert", "delete", "num", "exists", "first", "last", "next",
    "prev",
];

/// Methods of associative arrays that step through the keys.
const KEY_STEPS: &[&str] = &["first", "last", "next", "prev"];

/// A queue, dynamic or associative array named by an expression, ready for `Instr::Heap` /
/// `Instr::Assoc`: `slot` holds its handle.
#[derive(Clone, Copy)]
pub(crate) struct DynArr {
    pub slot: u32,
    pub kind: DynKind,
    pub elem: Elem,
    home: DynHome,
}

/// Where the handle of a `DynArr` lives.
#[derive(Clone, Copy)]
enum DynHome {
    /// a variable
    Sig(u32),
    /// the property at bit `off` of the object whose handle is in slot `h`
    Prop { h: u32, off: u32 },
}

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
        Elem { w, signed: signed || leaf == Leaf::Real, leaf, two_state, class: NONE }
    }

    /// `elem_of` for a declaration of data type `ty` (a class: its handles).
    pub(crate) fn elem_for(&self, kind: NetKind, w: u32, signed: bool, ty: Option<&ferrooxide_front::lower::SvType>, sid: usize) -> R<Elem> {
        if kind != NetKind::Class {
            return Ok(Self::elem_of(kind, w, signed));
        }
        let name = ty.and_then(|t| t.class.clone()).unwrap_or_default();
        let c = self.find_class(&name, sid).ok_or_else(|| self.mkerr(format!("unknown class '{name}'")))?;
        Ok(Elem { w: 64, signed: false, leaf: Leaf::Bits, two_state: true, class: c })
    }

    /// Declare `name` in scope `sid` as a queue (or dynamic array) of `elem` (full name
    /// `full`): a 64-bit handle, the empty array.
    pub(crate) fn new_dyn(&mut self, full: String, name: &str, elem: Elem, h: &HeapKind, sid: usize) -> R<u32> {
        let kind = self.dyn_kind(h, sid)?;
        Ok(self.new_dyn_kind(full, name, elem, kind, sid))
    }

    /// The kind of an array dimension (an associative array's key type evaluated in `sid`).
    pub(crate) fn dyn_kind(&mut self, h: &HeapKind, sid: usize) -> R<DynKind> {
        Ok(match h {
            HeapKind::Queue(None) => DynKind::Queue(0),
            HeapKind::Queue(Some(max)) => {
                // `[$:N]`: at most N + 1 elements
                if !self.is_const(max, sid) {
                    return self.err("queue bound must be constant.");
                }
                let c = self.const_eval(max, sid)?;
                match c.to_i64() {
                    None => return self.err("queue bound must be defined."),
                    Some(v) if v < 0 => return self.err(format!("queue bound must be positive ({v}).")),
                    Some(v) => DynKind::Queue(v as u32 + 1),
                }
            }
            HeapKind::Dynamic => DynKind::Darray,
            // `[*]`: integer keys
            HeapKind::Assoc(None) => DynKind::Assoc(KeyKind::Int { w: 64, signed: false }),
            HeapKind::Assoc(Some(k)) if k.kind == NetKind::String => DynKind::Assoc(KeyKind::Str),
            HeapKind::Assoc(Some(k)) => {
                let w = self.const_int(&k.width(), sid)?;
                if !(1..=64).contains(&w) {
                    return self.err("associative array keys wider than 64 bits are not supported");
                }
                DynKind::Assoc(KeyKind::Int { w: w as u32, signed: k.signed })
            }
        })
    }

    /// `new_dyn` of a known kind.
    pub(crate) fn new_dyn_kind(&mut self, full: String, name: &str, elem: Elem, kind: DynKind, sid: usize) -> u32 {
        let s = self.new_signal(full, 64, false, 0, 63, 0, 0, SigKind::Reg);
        let slot = self.sig(s).slot as usize;
        self.prog.mem[slot] = 0;
        self.prog.mem[slot + 1] = 0;
        self.sigs.types[s as usize] = Type::Dyn { kind, elem };
        if let DynKind::Assoc(KeyKind::Str) = kind {
            self.intern(b"");
        }
        self.sigs.has_dyn = true;
        if elem.leaf == Leaf::Str {
            self.intern(b"");
        }
        self.scopes[sid].syms.insert(name.to_string(), Sym::Sig(s));
        s
    }

    /// A heap operation on an array that is no variable's (a new or copied one).
    fn heap_op(&mut self, op: HeapOp, d: u32, a: u32, b: u32, c: u32, elem: Elem) {
        self.heap_op_t(op, d, a, b, c, elem, DynKind::Darray);
    }

    fn heap_op_t(&mut self, op: HeapOp, d: u32, a: u32, b: u32, c: u32, elem: Elem, kind: DynKind) {
        let ek = match elem.leaf {
            Leaf::Bits => rt::ElemKind::Vec,
            Leaf::Real => rt::ElemKind::Real,
            Leaf::Str => rt::ElemKind::Str,
        };
        let (queue, bound) = match kind {
            DynKind::Queue(b) => (true, b),
            _ => (false, 0),
        };
        let t = rt::HeapTy { elem: ek, queue, bound, x: elem.x() };
        let at = self.loc_id();
        self.emit(Instr::Heap { op, d, a, b, c, w: elem.w, t, at });
    }

    /// A copy of the array whose handle is in slot `h`: the slot of the new handle.
    pub(crate) fn dyn_copy(&mut self, h: u32, kind: DynKind, elem: Elem) -> u32 {
        // (also for an associative array)
        let d = self.tmp(64);
        self.heap_op_t(HeapOp::Copy, d, h, NONE, NONE, elem, kind);
        d
    }

    /// A heap operation on array `a`.
    fn arr_op(&mut self, a: &DynArr, op: HeapOp, d: u32, b: u32, c: u32) {
        self.heap_op_t(op, d, a.slot, b, c, a.elem, a.kind);
    }

    /// A property of a class object holding a queue, dynamic or associative array
    /// (`obj.q`): (the handle expression, the property's offset, kind and element type).
    fn dyn_prop<'e>(&mut self, e: &'e Expr, sid: usize) -> R<Option<(&'e Expr, u32, DynKind, Elem)>> {
        if self.classes.list.is_empty() {
            return Ok(None);
        }
        let Expr::Member(b, m) = e else { return Ok(None) };
        let Some(c) = self.handle_class(b, sid)? else { return Ok(None) };
        if self.class_const(c, m).is_some() {
            return Ok(None);
        }
        let p = self.prop(c, m)?;
        Ok(match p.ty {
            Type::Dyn { kind, elem } => Some((&**b, p.off, kind, elem)),
            _ => None,
        })
    }

    /// The array `e` names (a variable or a property), None if it names none. A property's
    /// handle is read into a temporary; `dyn_done` writes it back after a change.
    pub(crate) fn dyn_arr(&mut self, e: &Expr, sid: usize) -> R<Option<DynArr>> {
        if let Some((s, elem)) = self.dyn_var(e, sid) {
            let kind = match self.sig_type(s) {
                Type::Dyn { kind, .. } => kind,
                _ => DynKind::Darray,
            };
            return Ok(Some(DynArr { slot: self.sig(s).slot, kind, elem, home: DynHome::Sig(s) }));
        }
        let Some((b, off, kind, elem)) = self.dyn_prop(e, sid)? else { return Ok(None) };
        let h = self.handle_value(b, sid)?;
        let slot = self.tmp(64);
        self.emit(Instr::Obj { op: rt::ObjOp::Get, d: slot, a: h, b: NONE, off, w: 64 });
        Ok(Some(DynArr { slot, kind, elem, home: DynHome::Prop { h, off } }))
    }

    /// After array `a` changed: wake the variable's waiters, or write the property's handle
    /// back (an empty array gets an object when it changes).
    fn dyn_done(&mut self, a: &DynArr) {
        match a.home {
            DynHome::Sig(s) => {
                self.emit(Instr::Touch { sig: s });
            }
            DynHome::Prop { h, off } => {
                self.emit(Instr::Obj { op: rt::ObjOp::Set, d: NONE, a: h, b: a.slot, off, w: 64 });
            }
        }
    }

    /// `base.m` / `base.m(args)` where `base` is an array and `m` one of its methods:
    /// (base, method, arguments).
    fn dyn_method_expr<'e>(&mut self, e: &'e Expr, sid: usize) -> R<Option<(Expr, &'e str, &'e [Expr])>> {
        let (o, m, args): (&Expr, &str, &[Expr]) = match e {
            Expr::MethodCall(o, m, args) => (o, m, args),
            Expr::Member(o, m) => (o, m, &[]),
            Expr::Call(name, args) => match self.dyn_method(name, sid) {
                Some((base, _, _, m)) => {
                    let m = &name[name.len() - m.len()..];
                    return Ok(Some((base, m, args)));
                }
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
        if !METHODS.contains(&m) || self.dyn_arr_type(o, sid)?.is_none() {
            return Ok(None);
        }
        Ok(Some((o.clone(), m, args)))
    }

    /// The current source location for run-time warnings (`Program::locs`).
    fn loc_id(&mut self) -> u32 {
        let l = format!("{}:{}", self.file, self.line);
        if self.prog.locs.last() != Some(&l) {
            self.prog.locs.push(l);
        }
        self.prog.locs.len() as u32 - 1
    }

    fn assoc_op(&mut self, op: AssocOp, d: u32, a: u32, b: u32, c: u32, elem: Elem, key: KeyKind) {
        self.emit(Instr::Assoc { op, d, a, b, c, w: elem.w, key, x: elem.x() });
    }

    /// The key type of associative array `a` (None: not one).
    fn assoc_key(a: &DynArr) -> Option<KeyKind> {
        match a.kind {
            DynKind::Assoc(k) => Some(k),
            _ => None,
        }
    }

    /// A key of an associative array: the slot of its value.
    fn key_value(&mut self, e: &Expr, key: KeyKind, sid: usize) -> R<u32> {
        match key {
            KeyKind::Str => self.str_expr(e, sid),
            KeyKind::Int { w, signed } => self.expr(e, sid, w, signed),
        }
    }

    fn key_width(key: KeyKind) -> u32 {
        match key {
            KeyKind::Str => 64,
            KeyKind::Int { w, .. } => w,
        }
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

    /// The kind and element type of the array `e` names, without code.
    pub(crate) fn dyn_arr_type(&mut self, e: &Expr, sid: usize) -> R<Option<(DynKind, Elem)>> {
        if let Some((s, _)) = self.dyn_var(e, sid) {
            if let Type::Dyn { kind, elem } = self.sig_type(s) {
                return Ok(Some((kind, elem)));
            }
        }
        Ok(self.dyn_prop(e, sid)?.map(|(_, _, k, el)| (k, el)))
    }

    /// The type of an element read, a method call or a slice of a queue or dynamic array.
    pub(crate) fn dyn_type_of(&mut self, e: &Expr, sid: usize) -> R<Option<Type>> {
        match e {
            Expr::Index(a, _) => Ok(self.dyn_arr_type(a, sid)?.map(|(_, el)| el.ty())),
            Expr::Range(a, ..) => Ok(self.dyn_arr_type(a, sid)?.map(|(kind, elem)| Type::Dyn { kind, elem })),
            _ => {
                let Some((base, m, _)) = self.dyn_method_expr(e, sid)? else { return Ok(None) };
                let Some((_, elem)) = self.dyn_arr_type(&base, sid)? else { return Ok(None) };
                Ok(Some(match m {
                    "size" | "num" | "exists" => Type::vec(32, true),
                    m if KEY_STEPS.contains(&m) => Type::vec(32, true),
                    "pop_back" | "pop_front" => elem.ty(),
                    _ => return self.err(format!("'{m}' gives no value")),
                }))
            }
        }
    }

    /// `e` with `$` replaced by the last index of the queue `q`.
    fn subst_last(e: &Expr, q: &Expr) -> Expr {
        let b = |x: &Expr| Box::new(Self::subst_last(x, q));
        match e {
            Expr::Last => {
                let size = match q {
                    Expr::Ident(n) => Expr::Call(format!("{n}.size"), vec![]),
                    Expr::Hier(p) => Expr::Call(format!("{}.size", p.join(".")), vec![]),
                    q => Expr::MethodCall(Box::new(q.clone()), "size".into(), vec![]),
                };
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
        if let Expr::Index(a, i) = e {
            if self.dyn_arr_type(a, sid)?.is_none() {
                return Ok(None);
            }
            // (the index first: a property's handle is read last)
            let ix = match self.dyn_arr_type(a, sid)? {
                Some((DynKind::Assoc(key), _)) => self.key_value(i, key, sid)?,
                _ => self.dyn_index(a, i, sid)?,
            };
            let arr = self.dyn_arr(a, sid)?.unwrap();
            let elem = arr.elem;
            let d = self.tmp(elem.w);
            match Self::assoc_key(&arr) {
                Some(key) => self.assoc_op(AssocOp::Get, d, arr.slot, ix, NONE, elem, key),
                None => self.arr_op(&arr, HeapOp::Get, d, ix, NONE),
            }
            return Ok(Some((d, elem.ty())));
        }
        let Some((base, m, args)) = self.dyn_method_expr(e, sid)? else { return Ok(None) };
        let Some(arr) = self.dyn_arr(&base, sid)? else { return Ok(None) };
        let elem = arr.elem;
        if let Some(key) = Self::assoc_key(&arr) {
            return self.assoc_value(&arr, m, args, key, sid).map(Some);
        }
        match m {
            "size" => {
                let d = self.tmp(32);
                self.arr_op(&arr, HeapOp::Size, d, NONE, NONE);
                Ok(Some((d, Type::vec(32, true))))
            }
            "pop_back" | "pop_front" => {
                if !args.is_empty() {
                    return self.err(format!("{m}() takes no arguments"));
                }
                let d = self.tmp(elem.w);
                let op = if m == "pop_back" { HeapOp::PopBack } else { HeapOp::PopFront };
                self.arr_op(&arr, op, d, NONE, NONE);
                self.dyn_done(&arr);
                Ok(Some((d, elem.ty())))
            }
            _ => self.err(format!("'{m}' gives no value")),
        }
    }

    /// A method of associative array `a` that gives a value: (slot, type).
    fn assoc_value(&mut self, a: &DynArr, m: &str, args: &[Expr], key: KeyKind, sid: usize) -> R<(u32, Type)> {
        let (slot, elem) = (a.slot, a.elem);
        let d = self.tmp(32);
        match m {
            "num" | "size" => self.assoc_op(AssocOp::Num, d, slot, NONE, NONE, elem, key),
            "exists" => {
                let [k] = args else { return self.err("exists() takes 1 argument") };
                let k = self.key_value(k, key, sid)?;
                self.assoc_op(AssocOp::Exists, d, slot, k, NONE, elem, key);
            }
            m if KEY_STEPS.contains(&m) => {
                // the key variable (a `ref` argument) gets the key found
                let [kv] = args else { return self.err(format!("{m}() takes 1 argument")) };
                let cur = match m {
                    "next" | "prev" => self.key_value(kv, key, sid)?,
                    _ => NONE,
                };
                let kw = Self::key_width(key);
                let out = self.tmp(kw);
                if cur != NONE {
                    self.emit(Instr::Copy { d: out, s: cur, w: kw });
                }
                let op = match m {
                    "first" => AssocOp::First,
                    "last" => AssocOp::Last,
                    "next" => AssocOp::Next,
                    _ => AssocOp::Prev,
                };
                self.assoc_op(op, d, slot, cur, out, elem, key);
                // (unchanged when there is no such key)
                let found = self.tmp(1);
                self.emit(Instr::Red { op: rt::RedOp::Bool, d: found, s: d, w: 32 });
                let j = self.emit(Instr::Jz { c: found, w: 1, to: 0 });
                self.store_value(&stmt::Lhs::Expr(kv, sid), out, kw, rt::StoreMode::Blocking)?;
                let h = self.here();
                self.patch(j, h);
            }
            _ => return self.err(format!("'{m}' gives no value")),
        }
        Ok((d, Type::vec(32, true)))
    }

    /// An element value (`e` of the element type) for storing into an array of `elem`.
    fn elem_value(&mut self, e: &Expr, elem: Elem, sid: usize) -> R<u32> {
        self.value_for(e, sid, elem.w, elem.ty(), None)
    }

    /// A queue or dynamic array value: the slot of its handle and whether the array is new
    /// (else it belongs to a variable and is copied where it is stored).
    fn dyn_rhs(&mut self, e: &Expr, elem: Elem, sid: usize) -> R<(u32, bool)> {
        self.dyn_rhs_k(e, elem, false, sid)
    }

    /// `dyn_rhs` for a target that is an associative array (`assoc`) or not.
    fn dyn_rhs_k(&mut self, e: &Expr, elem: Elem, assoc: bool, sid: usize) -> R<(u32, bool)> {
        if let Some(a) = self.dyn_arr(e, sid)? {
            let el = a.elem;
            if el != elem && (el.w != elem.w || el.leaf != elem.leaf) {
                return self.err("the arrays have different element types");
            }
            return Ok((a.slot, false));
        }
        match e {
            Expr::NewArray(..) | Expr::Concat(_) | Expr::Pattern(..) | Expr::Range(..) if assoc => {
                self.err("expected an associative array value")
            }
            Expr::NewArray(n, init) => {
                let size = self.expr(n, sid, 32, true)?;
                // `new[n](v)` with a single real or string: every element is v (as in the
                // reference; the standard wants an array, ICARUS_NOTES 16)
                let (fill, src) = match init {
                    Some(i) if !matches!(**i, Expr::Pattern(..) | Expr::Concat(_)) && !matches!(self.type_of(i, sid)?, Type::Dyn { .. }) => {
                        if elem.leaf == Leaf::Bits {
                            return self.err("`new[n](v)` with a value that is not an array is not supported for vector elements");
                        }
                        (self.elem_value(i, elem, sid)?, NONE)
                    }
                    Some(i) => (NONE, self.dyn_rhs(i, elem, sid)?.0),
                    None => (NONE, NONE),
                };
                let d = self.tmp(64);
                self.heap_op(HeapOp::New, d, fill, size, src, elem);
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
            Expr::Range(a, lo, hi) if self.dyn_arr_type(a, sid)?.is_some() => {
                let (lo, hi) = (self.dyn_index(a, lo, sid)?, self.dyn_index(a, hi, sid)?);
                let arr = self.dyn_arr(a, sid)?.unwrap();
                let d = self.tmp(64);
                self.heap_op(HeapOp::Slice, d, arr.slot, lo, hi, arr.elem);
                Ok((d, true))
            }
            Expr::Cond(c, x, y) => {
                // (both arrays are made; the condition picks one)
                let k = if assoc { DynKind::Assoc(KeyKind::Str) } else { DynKind::Queue(0) };
                let x = self.dyn_owned(x, k, elem, sid)?;
                let y = self.dyn_owned(y, k, elem, sid)?;
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
    pub(crate) fn dyn_owned(&mut self, e: &Expr, kind: DynKind, elem: Elem, sid: usize) -> R<u32> {
        let (h, fresh) = self.dyn_rhs_k(e, elem, matches!(kind, DynKind::Assoc(_)), sid)?;
        let h = if fresh {
            h
        } else {
            let d = self.tmp(64);
            self.heap_op(HeapOp::Copy, d, h, NONE, NONE, elem);
            d
        };
        // a bounded queue keeps its first elements
        if let DynKind::Queue(b) = kind {
            if b > 0 {
                self.heap_op_t(HeapOp::Truncate, NONE, h, NONE, NONE, elem, kind);
            }
        }
        Ok(h)
    }

    /// An assignment pattern with more elements than bounded queue `name` holds: the
    /// reference's compile-time warning and the pattern cut to the first elements.
    pub(crate) fn bounded_pattern(&mut self, rhs: &Expr, lhs: &Expr, sid: usize) -> Option<Expr> {
        let Expr::Pattern(None, items) = rhs else { return None };
        let (s, _) = self.dyn_var(lhs, sid)?;
        let Type::Dyn { kind: DynKind::Queue(b), .. } = self.sig_type(s) else { return None };
        if b == 0 || items.len() <= b as usize {
            return None;
        }
        let name = match lhs {
            Expr::Ident(n) => n.clone(),
            Expr::Hier(p) => p.last().cloned().unwrap_or_default(),
            _ => return None,
        };
        let w = format!(
            "{}:{}: Warning: Array pattern assignment has more elements ({}) than bounded queue '{name}' supports ({b}).\n         Only using first {b} elements.",
            self.file,
            self.line,
            items.len()
        );
        self.diag.warnings.push(w);
        Some(Expr::Pattern(None, items[..b as usize].to_vec()))
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
        let Expr::Index(a, _) = lhs else { return Ok(false) };
        let Some((_, elem)) = self.dyn_arr_type(a, sid)? else { return Ok(false) };
        let v = self.elem_value(rhs, elem, sid)?;
        self.dyn_elem_store(lhs, v, sid)
    }

    /// Store the element value in slot `v` to `q[i]` of a queue or dynamic array: returns
    /// false if `lhs` is no such element.
    pub(crate) fn dyn_elem_store(&mut self, lhs: &Expr, v: u32, sid: usize) -> R<bool> {
        let Expr::Index(a, i) = lhs else { return Ok(false) };
        let Some((kind, elem)) = self.dyn_arr_type(a, sid)? else { return Ok(false) };
        let ix = match kind {
            DynKind::Assoc(key) => self.key_value(i, key, sid)?,
            _ => self.dyn_index(a, i, sid)?,
        };
        let arr = self.dyn_arr(a, sid)?.unwrap();
        match Self::assoc_key(&arr) {
            Some(key) => self.assoc_op(AssocOp::Set, NONE, arr.slot, ix, v, elem, key),
            None => self.arr_op(&arr, HeapOp::Set, NONE, ix, v),
        }
        self.dyn_done(&arr);
        Ok(true)
    }

    /// `q.method(args);` of a queue or dynamic array: returns false if `name` is no such
    /// method.
    pub(crate) fn dyn_task(&mut self, name: &str, args: &[Expr], sid: usize) -> R<bool> {
        let Some((base, _, _, m)) = self.dyn_method(name, sid) else { return Ok(false) };
        let m = &name[name.len() - m.len()..];
        self.dyn_method_stmt(&base, m, args, sid)
    }

    /// `q.method(args);` as a method call (`obj.q`: an array property, `q`: an array of
    /// class handles): false if `obj` is no array.
    pub(crate) fn dyn_method_task(&mut self, obj: &Expr, m: &str, args: &[Expr], sid: usize) -> R<bool> {
        if !METHODS.contains(&m) || self.dyn_arr_type(obj, sid)?.is_none() {
            return Ok(false);
        }
        self.dyn_method_stmt(obj, m, args, sid)
    }

    /// Method `m` of array `base` called as a statement.
    fn dyn_method_stmt(&mut self, base: &Expr, m: &str, args: &[Expr], sid: usize) -> R<bool> {
        let Some((kind, elem)) = self.dyn_arr_type(base, sid)? else { return Ok(false) };
        if let DynKind::Assoc(key) = kind {
            if m == "delete" {
                let k = match args {
                    [] => NONE,
                    [k] => self.key_value(k, key, sid)?,
                    _ => return self.err("delete() takes at most 1 argument"),
                };
                let arr = self.dyn_arr(base, sid)?.unwrap();
                self.assoc_op(AssocOp::Delete, NONE, arr.slot, k, NONE, elem, key);
                self.dyn_done(&arr);
            } else {
                // a value method called as a statement (`aa.first(k);`)
                let arr = self.dyn_arr(base, sid)?.unwrap();
                self.assoc_value(&arr, m, args, key, sid)?;
            }
            return Ok(true);
        }
        let nargs = |c: &Self, n: usize| -> R<()> {
            if args.len() != n {
                return c.err(format!("{m}() takes {n} argument{}", if n == 1 { "" } else { "s" }));
            }
            Ok(())
        };
        let (op, b, c) = match m {
            "push_back" | "push_front" => {
                nargs(self, 1)?;
                let v = self.elem_value(&args[0], elem, sid)?;
                (if m == "push_back" { HeapOp::PushBack } else { HeapOp::PushFront }, NONE, v)
            }
            "insert" => {
                nargs(self, 2)?;
                let ix = self.dyn_index(base, &args[0], sid)?;
                let v = self.elem_value(&args[1], elem, sid)?;
                (HeapOp::Insert, ix, v)
            }
            "delete" => {
                let ix = match args {
                    [] => NONE,
                    [i] => self.dyn_index(base, i, sid)?,
                    _ => return self.err("delete() takes at most 1 argument"),
                };
                (HeapOp::Delete, ix, NONE)
            }
            // a value method called for its effect (pop) or ignored (size)
            _ => {
                self.dyn_value(&Expr::MethodCall(Box::new(base.clone()), m.to_string(), args.to_vec()), sid)?;
                return Ok(true);
            }
        };
        let arr = self.dyn_arr(base, sid)?.unwrap();
        self.arr_op(&arr, op, NONE, b, c);
        self.dyn_done(&arr);
        Ok(true)
    }

    /// `a == b` / `a != b` of queues or dynamic arrays (1 bit).
    pub(crate) fn dyn_compare(&mut self, ne: bool, a: &Expr, b: &Expr, elem: Elem, sid: usize) -> R<u32> {
        let assoc = |c: &mut Self, e: &Expr| -> R<bool> { Ok(matches!(c.dyn_arr_type(e, sid)?, Some((DynKind::Assoc(_), _)))) };
        if assoc(self, a)? || assoc(self, b)? {
            return self.err("associative arrays cannot be compared");
        }
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
            Type::Class(_) => self.resize(v, 64, w, false),
        })
    }
}
