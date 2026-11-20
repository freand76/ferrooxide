//! Array manipulation methods of queues and dynamic arrays (IEEE 1800-2017 7.12): the
//! reduction methods (`sum`, `product`, `and`, `or`, `xor`), the locator methods (`find*`,
//! `min`, `max`, `unique*`), all with an optional `with (e)` evaluated for each element,
//! and the ordering methods (`sort`, `rsort`, `reverse`, `shuffle`).
//!
//! The reductions and locators are loops in the process's code over the elements: the
//! iterator of `with` is a variable of a scope of its own (`$it<depth>`, its index
//! `$ix<depth>`, named so by the lowering), set before `e` is evaluated for an element. A
//! locator builds a new queue. The ordering methods reorder the array in place
//! (`HeapOp::Order`).

use crate::heap::DynArr;
use crate::types::{DynKind, Elem, Leaf, Type};
use crate::*;
use ferrooxide_rt::{HeapOp, OrderOp};

pub(crate) const REDUCE: &[&str] = &["sum", "product", "and", "or", "xor"];
pub(crate) const LOCATE: &[&str] =
    &["find", "find_index", "find_first", "find_first_index", "find_last", "find_last_index", "min", "max", "unique", "unique_index"];
pub(crate) const ORDER: &[&str] = &["sort", "rsort", "reverse", "shuffle"];

/// An array method call: the array, the method and its `with` expression (with the depth
/// that names its iterator).
pub(crate) struct ArrMethod<'e> {
    pub arr: Expr,
    pub m: String,
    pub with: Option<(&'e Expr, usize)>,
}

/// An int element (the indexes `*_index` give).
const INT: Elem = Elem { w: 32, signed: true, leaf: Leaf::Bits, two_state: true, class: NONE };

impl<'a> Compiler<'a> {
    /// `e` as a reduction or locator method call of a queue or dynamic array.
    pub(crate) fn arr_method<'e>(&mut self, e: &'e Expr, sid: usize) -> R<Option<ArrMethod<'e>>> {
        let (arr, m, with) = match e {
            Expr::ArrayWith { arr, method, depth, with } => ((**arr).clone(), method.clone(), Some((&**with, *depth))),
            _ => {
                // (`a.max` without parentheses: a hierarchical name)
                let call;
                let e = match self.dyn_hier_call(e, sid) {
                    Some(c) => {
                        call = c;
                        &call
                    }
                    None => e,
                };
                let Some((base, m, args)) = self.dyn_method_expr(e, sid)? else { return Ok(None) };
                if !REDUCE.contains(&m) && !LOCATE.contains(&m) {
                    return Ok(None);
                }
                if !args.is_empty() && !matches!(args, [Expr::EmptyArg]) {
                    return self.err(format!("'{m}' takes no arguments"));
                }
                (base, m.to_string(), None)
            }
        };
        let Some((kind, _)) = self.dyn_arr_type(&arr, sid)? else {
            return self.err(format!("array method '{m}' is supported only for queues and dynamic arrays"));
        };
        if matches!(kind, DynKind::Assoc(_)) {
            return self.err(format!("array method '{m}' of an associative array is not supported"));
        }
        if !REDUCE.contains(&m.as_str()) && !LOCATE.contains(&m.as_str()) {
            return self.err(format!("'{m}' has no value (a void method)"));
        }
        if m.starts_with("find") && with.is_none() {
            return self.err(format!("'{m}' needs a `with` expression"));
        }
        Ok(Some(ArrMethod { arr, m, with }))
    }

    /// The error for an array manipulation method of a fixed-size array (`p`: the array's
    /// name and the method), which is not supported; Ok if `p` is no such call.
    pub(crate) fn fixed_arr_method(&self, p: &[String], sid: usize) -> R<()> {
        let [base @ .., m] = p else { return Ok(()) };
        if base.is_empty() || ![REDUCE, LOCATE, ORDER].iter().any(|l| l.contains(&m.as_str())) {
            return Ok(());
        }
        let sym = if base.len() == 1 { self.lookup(&base[0], sid) } else { self.lookup_hier(base, sid) };
        match sym {
            Some(Sym::Sig(s)) if self.sig(s).depth > 0 => self.err(format!("array method '{m}' of a fixed-size array is not supported")),
            _ => Ok(()),
        }
    }

    /// The scope of the iterator of `with` expression `w` (in scope `sid`), and its element
    /// and index variables, made once.
    fn iter_scope(&mut self, w: &Expr, depth: usize, elem: Elem, sid: usize) -> (usize, u32, u32) {
        let key = (w as *const Expr, sid);
        if let Some(&s) = self.code.iter_scopes.get(&key) {
            return s;
        }
        let path = self.scopes[sid].name.clone();
        let file = self.file.clone();
        let isid = self.new_scope(path.clone(), Some(sid), file);
        let it = self.new_signal(format!("{path}.$it{depth}"), elem.w, elem.signed, 0, elem.w as i64 - 1, 0, 0, SigKind::Reg);
        self.prog.names.pop();
        {
            let info = &mut self.prog.signals[it as usize];
            info.real = elem.leaf == Leaf::Real;
            info.string = elem.leaf == Leaf::Str;
        }
        self.sigs.types[it as usize] = elem.ty();
        let ix = self.new_signal(format!("{path}.$ix{depth}"), 32, true, 0, 31, 0, 0, SigKind::Reg);
        self.prog.names.pop();
        self.scopes[isid].syms.insert(format!("$it{depth}"), Sym::Sig(it));
        self.scopes[isid].syms.insert(format!("$ix{depth}"), Sym::Sig(ix));
        let r = (isid, it, ix);
        self.code.iter_scopes.insert(key, r);
        r
    }

    /// The type of the `with` expression of `am` (None: no `with`).
    fn with_type(&mut self, am: &ArrMethod, elem: Elem, sid: usize) -> R<Option<Type>> {
        let Some((w, depth)) = am.with else { return Ok(None) };
        let (isid, _, _) = self.iter_scope(w, depth, elem, sid);
        self.type_of(w, isid).map(Some)
    }

    /// The type of the value of `am`.
    pub(crate) fn arr_method_type(&mut self, am: &ArrMethod, sid: usize) -> R<Type> {
        let (_, elem) = self.dyn_arr_type(&am.arr, sid)?.unwrap();
        let wt = self.with_type(am, elem, sid)?;
        Ok(if REDUCE.contains(&am.m.as_str()) {
            wt.unwrap_or(elem.ty())
        } else if am.m.ends_with("_index") {
            Type::Dyn { kind: DynKind::Queue(0), elem: INT }
        } else {
            Type::Dyn { kind: DynKind::Queue(0), elem }
        })
    }

    /// A loop over the elements of `a`: for each, the iterator variables (`iter`) are set
    /// and `body` gets the slots of the element and of its index.
    fn arr_loop(&mut self, a: &DynArr, iter: Option<(u32, u32)>, mut body: impl FnMut(&mut Self, u32, u32) -> R<()>) -> R<()> {
        let elem = a.elem;
        let size = self.tmp(32);
        self.emit(Instr::Heap { op: HeapOp::Size, d: size, a: a.slot, b: NONE, c: NONE, w: elem.w, t: self.heap_ty(a), at: 0 });
        let i = self.tmp(32);
        let zero = self.const_slot(&Const::zero(32));
        self.emit(Instr::Copy { d: i, s: zero, w: 32 });
        let top = self.here();
        let c = self.tmp(1);
        self.emit(Instr::Cmp { op: rt::CmpOp::LtS, d: c, a: i, b: size, w: 32 });
        let j = self.emit(Instr::Jz { c, w: 1, to: 0 });
        let el = self.tmp(elem.w);
        self.emit(Instr::Heap { op: HeapOp::Get, d: el, a: a.slot, b: i, c: NONE, w: elem.w, t: self.heap_ty(a), at: 0 });
        if let Some((it, ix)) = iter {
            let (its, ixs) = (self.sig(it).slot, self.sig(ix).slot);
            self.emit(Instr::Copy { d: its, s: el, w: elem.w });
            self.emit(Instr::Copy { d: ixs, s: i, w: 32 });
        }
        body(self, el, i)?;
        let one = self.const_slot(&Const::from_i64(1, 32, true));
        self.emit(Instr::Bin { op: rt::BinOp::Add, d: i, a: i, b: one, w: 32 });
        self.emit(Instr::Jmp { to: top });
        let end = self.here();
        self.patch(j, end);
        Ok(())
    }

    fn heap_ty(&self, a: &DynArr) -> rt::HeapTy {
        Self::heap_ty_of(a.elem, a.kind)
    }

    pub(crate) fn heap_ty_of(elem: Elem, kind: DynKind) -> rt::HeapTy {
        let ek = match elem.leaf {
            Leaf::Bits => rt::ElemKind::Vec,
            Leaf::Real => rt::ElemKind::Real,
            Leaf::Str => rt::ElemKind::Str,
        };
        let (queue, bound) = match kind {
            DynKind::Queue(b) => (true, b),
            _ => (false, 0),
        };
        rt::HeapTy { elem: ek, queue, bound, x: elem.x() }
    }

    /// A new empty queue of `elem`: the slot of its handle.
    fn empty_queue(&mut self) -> u32 {
        let d = self.tmp(64);
        let z = self.const_slot(&Const::zero(64));
        self.emit(Instr::Copy { d, s: z, w: 64 });
        d
    }

    fn push(&mut self, q: u32, v: u32, elem: Elem) {
        let t = Self::heap_ty_of(elem, DynKind::Queue(0));
        self.emit(Instr::Heap { op: HeapOp::PushBack, d: NONE, a: q, b: NONE, c: v, w: elem.w, t, at: 0 });
    }

    /// The value of reduction method `am`: (slot, type).
    pub(crate) fn arr_reduce(&mut self, am: &ArrMethod, sid: usize) -> R<(u32, Type)> {
        let t = self.arr_method_type(am, sid)?;
        let Type::Vec { w, signed } = t else {
            return self.err(format!("'{}' needs integral values", am.m));
        };
        let a = self.dyn_arr(&am.arr, sid)?.unwrap();
        let iter = am.with.map(|(we, depth)| self.iter_scope(we, depth, a.elem, sid));
        let init = match am.m.as_str() {
            "product" => Const::from_i64(1, w, false),
            "and" => Const::from_i64(-1, w, false),
            _ => Const::zero(w),
        };
        let acc = self.tmp(w);
        let c0 = self.const_slot(&init);
        self.emit(Instr::Copy { d: acc, s: c0, w });
        let op = match am.m.as_str() {
            "sum" => rt::BinOp::Add,
            "product" => rt::BinOp::Mul,
            "and" => rt::BinOp::And,
            "or" => rt::BinOp::Or,
            _ => rt::BinOp::Xor,
        };
        let with = am.with.map(|x| x.0);
        let elem = a.elem;
        self.arr_loop(&a, iter.map(|x| (x.1, x.2)), |c, el, _| {
            let v = match (with, iter) {
                (Some(we), Some((isid, _, _))) => c.expr(we, isid, w, signed)?,
                _ => c.resize(el, elem.w, w, elem.signed && signed),
            };
            c.emit(Instr::Bin { op, d: acc, a: acc, b: v, w });
            Ok(())
        })?;
        Ok((acc, t))
    }

    /// The queue that locator method `am` gives: the slot of its handle (a new array).
    pub(crate) fn arr_locate(&mut self, am: &ArrMethod, sid: usize) -> R<u32> {
        let a = self.dyn_arr(&am.arr, sid)?.unwrap();
        let elem = a.elem;
        let iter = am.with.map(|(we, depth)| self.iter_scope(we, depth, elem, sid));
        let with = am.with.map(|x| x.0);
        let m = am.m.clone();
        let index = m.ends_with("_index");
        let out_elem = if index { INT } else { elem };
        let q = self.empty_queue();
        // the element or index a match gives
        let pick = |el: u32, i: u32| if index { i } else { el };
        match m.trim_end_matches("_index") {
            "find" | "find_first" | "find_last" => {
                let (we, isid) = (with.unwrap(), iter.unwrap().0);
                // find_first: until the first match; find_last: the last match, pushed after
                let more = self.tmp(1);
                let one = self.const_slot(&Const::from_i64(1, 1, false));
                self.emit(Instr::Copy { d: more, s: one, w: 1 });
                let last = self.tmp(out_elem.w);
                let mode = m.trim_end_matches("_index").to_string();
                self.arr_loop(&a, iter.map(|x| (x.1, x.2)), |c, el, i| {
                    let (t, tw) = c.truth(we, isid)?;
                    let j = c.emit(Instr::Jz { c: t, w: tw, to: 0 });
                    let v = pick(el, i);
                    match mode.as_str() {
                        "find" => c.push(q, v, out_elem),
                        "find_first" => {
                            let j2 = c.emit(Instr::Jz { c: more, w: 1, to: 0 });
                            c.push(q, v, out_elem);
                            let z = c.const_slot(&Const::zero(1));
                            c.emit(Instr::Copy { d: more, s: z, w: 1 });
                            let h = c.here();
                            c.patch(j2, h);
                        }
                        _ => {
                            c.emit(Instr::Copy { d: last, s: v, w: out_elem.w });
                            let z = c.const_slot(&Const::zero(1));
                            c.emit(Instr::Copy { d: more, s: z, w: 1 });
                        }
                    }
                    let h = c.here();
                    c.patch(j, h);
                    Ok(())
                })?;
                if mode == "find_last" {
                    // (`more` is 0 once something matched)
                    let none = self.tmp(1);
                    self.emit(Instr::Un { op: rt::UnOp::Not, d: none, s: more, w: 1 });
                    let j = self.emit(Instr::Jz { c: none, w: 1, to: 0 });
                    self.push(q, last, out_elem);
                    let h = self.here();
                    self.patch(j, h);
                }
            }
            "min" | "max" => {
                // the key compared: the `with` value or the element
                let kt = match with {
                    Some(we) => self.type_of(we, iter.unwrap().0)?,
                    None => elem.ty(),
                };
                let (kw, ks) = match kt {
                    Type::Vec { w, signed } => (w, signed),
                    Type::Str => (64, false),
                    _ => return self.err(format!("'{m}' of real values is not supported")),
                };
                let best = self.tmp(kw);
                let best_el = self.tmp(elem.w);
                let none = self.tmp(1);
                let one = self.const_slot(&Const::from_i64(1, 1, false));
                self.emit(Instr::Copy { d: none, s: one, w: 1 });
                let max = m == "max";
                self.arr_loop(&a, iter.map(|x| (x.1, x.2)), |c, el, _| {
                    let k = match (with, iter) {
                        (Some(we), Some((isid, _, _))) => c.value_for(we, isid, kw, kt, None)?,
                        _ => el,
                    };
                    let better = c.tmp(1);
                    let (x, y) = if max { (best, k) } else { (k, best) };
                    if kt == Type::Str {
                        c.emit(Instr::Str { op: rt::StrOp::Cmp(rt::CmpOp::Lt), d: better, a: x, b: y, c: NONE, w: 1 });
                    } else {
                        let op = if ks { rt::CmpOp::LtS } else { rt::CmpOp::Lt };
                        c.emit(Instr::Cmp { op, d: better, a: x, b: y, w: kw });
                    }
                    c.emit(Instr::Bin { op: rt::BinOp::Or, d: better, a: better, b: none, w: 1 });
                    let j = c.emit(Instr::Jz { c: better, w: 1, to: 0 });
                    c.emit(Instr::Copy { d: best, s: k, w: kw });
                    c.emit(Instr::Copy { d: best_el, s: el, w: elem.w });
                    let z = c.const_slot(&Const::zero(1));
                    c.emit(Instr::Copy { d: none, s: z, w: 1 });
                    let h = c.here();
                    c.patch(j, h);
                    Ok(())
                })?;
                let found = self.tmp(1);
                self.emit(Instr::Un { op: rt::UnOp::Not, d: found, s: none, w: 1 });
                let j = self.emit(Instr::Jz { c: found, w: 1, to: 0 });
                self.push(q, best_el, elem);
                let h = self.here();
                self.patch(j, h);
            }
            _ => {
                // unique: the first element of each value (or `with` value) seen
                let kt = match with {
                    Some(we) => self.type_of(we, iter.unwrap().0)?,
                    None => elem.ty(),
                };
                let key_elem = match kt {
                    _ if with.is_none() => elem,
                    Type::Vec { w, signed } => Elem { w, signed, leaf: Leaf::Bits, two_state: false, class: NONE },
                    Type::Str => Elem { w: 64, signed: false, leaf: Leaf::Str, two_state: true, class: NONE },
                    Type::Real => Elem { w: 64, signed: false, leaf: Leaf::Real, two_state: true, class: NONE },
                    _ => return self.err(format!("'{m}' with this `with` expression is not supported")),
                };
                let seen = self.empty_queue();
                let kt_q = Self::heap_ty_of(key_elem, DynKind::Queue(0));
                self.arr_loop(&a, iter.map(|x| (x.1, x.2)), |c, el, i| {
                    let k = match (with, iter) {
                        (Some(we), Some((isid, _, _))) => c.value_for(we, isid, key_elem.w, kt, None)?,
                        _ => el,
                    };
                    let has = c.tmp(1);
                    c.emit(Instr::Heap { op: HeapOp::Contains, d: has, a: seen, b: NONE, c: k, w: key_elem.w, t: kt_q, at: 0 });
                    let new = c.tmp(1);
                    c.emit(Instr::Un { op: rt::UnOp::Not, d: new, s: has, w: 1 });
                    let j = c.emit(Instr::Jz { c: new, w: 1, to: 0 });
                    c.push(seen, k, key_elem);
                    c.push(q, pick(el, i), out_elem);
                    let h = c.here();
                    c.patch(j, h);
                    Ok(())
                })?;
                // (the keys are dropped)
                self.emit(Instr::Heap { op: HeapOp::Free, d: NONE, a: seen, b: NONE, c: NONE, w: key_elem.w, t: kt_q, at: 0 });
            }
        }
        Ok(q)
    }

    /// `arr.sort with (e);` / `arr.rsort with (e);`: false if `e` is no ordering method
    /// with `with`.
    pub(crate) fn arr_order_with(&mut self, e: &Expr, sid: usize) -> R<bool> {
        let Expr::ArrayWith { arr, method: m, depth, with: we } = e else { return Ok(false) };
        if !ORDER.contains(&m.as_str()) {
            return Ok(false);
        }
        if !m.ends_with("sort") {
            return self.err(format!("'{m}' takes no `with` expression"));
        }
        let Some(a) = self.dyn_arr(arr, sid)? else {
            return self.err(format!("array method '{m}' is supported only for queues and dynamic arrays"));
        };
        if matches!(a.kind, DynKind::Assoc(_)) {
            return self.err(format!("'{m}' is not a method of an associative array"));
        }
        let (isid, it, ix) = self.iter_scope(we, *depth, a.elem, sid);
        let kt = self.type_of(we, isid)?;
        let key = match kt {
            Type::Vec { w, signed } => Elem { w, signed, leaf: Leaf::Bits, two_state: false, class: NONE },
            Type::Str => Elem { w: 64, signed: false, leaf: Leaf::Str, two_state: true, class: NONE },
            Type::Real => Elem { w: 64, signed: false, leaf: Leaf::Real, two_state: true, class: NONE },
            _ => return self.err(format!("'{m}' with this `with` expression is not supported")),
        };
        // the key of each element, then the elements reordered by them
        let keys = self.empty_queue();
        self.arr_loop(&a, Some((it, ix)), |c, _, _| {
            let k = c.value_for(we, isid, key.w, kt, None)?;
            c.push(keys, k, key);
            Ok(())
        })?;
        let ek = Self::heap_ty_of(key, DynKind::Queue(0)).elem;
        let o = OrderOp::SortBy { desc: m == "rsort", signed: key.signed, key: ek, kw: key.w };
        self.emit(Instr::Heap { op: HeapOp::Order(o), d: NONE, a: a.slot, b: keys, c: NONE, w: a.elem.w, t: self.heap_ty(&a), at: 0 });
        let kt_q = Self::heap_ty_of(key, DynKind::Queue(0));
        self.emit(Instr::Heap { op: HeapOp::Free, d: NONE, a: keys, b: NONE, c: NONE, w: key.w, t: kt_q, at: 0 });
        self.dyn_done(&a);
        Ok(true)
    }

    /// `arr.sort` and the other ordering methods as a statement: true if `m` is one.
    pub(crate) fn arr_order(&mut self, a: &DynArr, m: &str, args: &[Expr]) -> R<bool> {
        if !ORDER.contains(&m) {
            return Ok(false);
        }
        if !args.is_empty() && !matches!(args, [Expr::EmptyArg]) {
            return self.err(format!("'{m}' takes no arguments"));
        }
        if matches!(a.kind, DynKind::Assoc(_)) {
            return self.err(format!("'{m}' is not a method of an associative array"));
        }
        if a.elem.class != NONE && m.ends_with("sort") {
            return self.err(format!("'{m}' of class handles needs a `with` expression"));
        }
        let o = match m {
            "sort" => OrderOp::Sort { desc: false, signed: a.elem.signed },
            "rsort" => OrderOp::Sort { desc: true, signed: a.elem.signed },
            "reverse" => OrderOp::Reverse,
            _ => OrderOp::Shuffle,
        };
        self.emit(Instr::Heap { op: HeapOp::Order(o), d: NONE, a: a.slot, b: NONE, c: NONE, w: a.elem.w, t: self.heap_ty(a), at: 0 });
        Ok(true)
    }
}
