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
//!
//! A one-dimensional fixed-size array takes the same paths through a temporary queue: its
//! elements are copied into one (also for `q = f`), and an ordering method or `f = q`
//! stores the queue's elements back (`FixedArr`).

use crate::heap::DynArr;
use crate::types::{DynKind, Elem, Leaf, Type};
use crate::*;
use ferrooxide_rt::{HeapOp, OrderOp, StoreMode};

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

/// A whole one-dimensional fixed-size array (`sig`) of vectors, reals, strings or class
/// handles: `n` elements, declared with a descending range (`desc`).
#[derive(Clone, Copy)]
pub(crate) struct FixedArr {
    pub sig: u32,
    pub n: u32,
    pub desc: bool,
    pub elem: Elem,
}

/// An int element (the indexes `*_index` give).
const INT: Elem = Elem { w: 32, signed: true, leaf: Leaf::Bits, two_state: true, class: NONE, nested: NONE };

impl<'a> Compiler<'a> {
    /// `e` as a reduction or locator method call of a queue or dynamic array.
    pub(crate) fn arr_method<'e>(&mut self, e: &'e Expr, sid: usize) -> R<Option<ArrMethod<'e>>> {
        let (arr, m, with) = match &e.kind {
            ExprKind::ArrayWith { arr, method, depth, with } => ((**arr).clone(), method.clone(), Some((&**with, *depth))),
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
                let (base, m, nargs) = match self.dyn_method_expr(e, sid)? {
                    Some((base, m, args)) => (base, m.to_string(), args.len() - matches!(args, [Expr { kind: ExprKind::EmptyArg, .. }]) as usize),
                    None => match self.fixed_method_expr(e, sid)? {
                        Some((base, m, args)) => (base, m, args.len() - matches!(args[..], [Expr { kind: ExprKind::EmptyArg, .. }]) as usize),
                        None => return Ok(None),
                    },
                };
                if !REDUCE.contains(&m.as_str()) && !LOCATE.contains(&m.as_str()) {
                    return Ok(None);
                }
                if nargs != 0 {
                    return self.err(format!("'{m}' takes no arguments"));
                }
                (base, m, None)
            }
        };
        let Some((kind, _)) = self.method_arr_type(&arr, sid)? else {
            return self.err(format!("array method '{m}' is not supported for this array (only for queues, dynamic arrays and one-dimensional fixed-size arrays of vectors, reals, strings or class handles)"));
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

    /// The error for an array manipulation method of a fixed-size array that `fixed_arr`
    /// does not take (`p`: the array's name and the method); Ok if `p` is no such call.
    pub(crate) fn fixed_arr_method(&self, p: &[String], sid: usize) -> R<()> {
        let [base @ .., m] = p else { return Ok(()) };
        if base.is_empty() || ![REDUCE, LOCATE, ORDER].iter().any(|l| l.contains(&m.as_str())) {
            return Ok(());
        }
        let sym = if base.len() == 1 { self.lookup(&base[0], sid) } else { self.lookup_hier(base, sid) };
        match sym {
            Some(Sym::Sig(s)) if self.sig(s).depth > 0 => self.err(format!("array method '{m}' of a multi-dimensional array or an array of structs is not supported")),
            _ => Ok(()),
        }
    }

    /// The scope of the iterator of `with` expression `w` (in scope `sid`) over the elements
    /// of `arr`, and its element and index variables, made once.
    fn iter_scope(&mut self, w: &Expr, depth: usize, elem: Elem, arr: &Expr, sid: usize) -> R<(usize, u32, u32)> {
        let key = (w as *const Expr, sid);
        if let Some(&s) = self.code.iter_scopes.get(&key) {
            return Ok(s);
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
        // the elements of a queue of structs: the iterator has their members
        if let Some(l) = self.dyn_var(arr, sid).and_then(|(s, _)| self.sigs.elem_layout.get(&s).cloned()) {
            self.set_layout(it, &format!("$it{depth}"), Some((*l).clone()), isid)?;
        }
        self.code.iter_scopes.insert(key, r);
        Ok(r)
    }

    /// The type of the `with` expression of `am` (None: no `with`).
    fn with_type(&mut self, am: &ArrMethod, elem: Elem, sid: usize) -> R<Option<Type>> {
        let Some((w, depth)) = am.with else { return Ok(None) };
        let (isid, _, _) = self.iter_scope(w, depth, elem, &am.arr, sid)?;
        self.type_of(w, isid).map(Some)
    }

    /// The type of the value of `am`.
    pub(crate) fn arr_method_type(&mut self, am: &ArrMethod, sid: usize) -> R<Type> {
        let (_, elem) = self.method_arr_type(&am.arr, sid)?.unwrap();
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
        // the element's index: of the fixed-size array a copy was made of
        let index = match a.index_map() {
            None => i,
            Some((left, desc)) => {
                let x = self.tmp(32);
                let l = self.const_slot(&Const::from_i64(left, 32, true));
                let op = if desc { rt::BinOp::Sub } else { rt::BinOp::Add };
                self.emit(Instr::Bin { op, d: x, a: l, b: i, w: 32 });
                x
            }
        };
        if let Some((it, ix)) = iter {
            let (its, ixs) = (self.sig(it).slot, self.sig(ix).slot);
            self.emit(Instr::Copy { d: its, s: el, w: elem.w });
            self.emit(Instr::Copy { d: ixs, s: index, w: 32 });
        }
        body(self, el, index)?;
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
        let (a, fixed) = self.method_arr(&am.arr, sid)?;
        let iter = am.with.map(|(we, depth)| self.iter_scope(we, depth, a.elem, &am.arr, sid)).transpose()?;
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
        self.method_arr_done(&a, fixed);
        Ok((acc, t))
    }

    /// The queue that locator method `am` gives: the slot of its handle (a new array).
    pub(crate) fn arr_locate(&mut self, am: &ArrMethod, sid: usize) -> R<u32> {
        let (a, fixed) = self.method_arr(&am.arr, sid)?;
        let elem = a.elem;
        let iter = am.with.map(|(we, depth)| self.iter_scope(we, depth, elem, &am.arr, sid)).transpose()?;
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
                    Type::Vec { w, signed } => Elem { w, signed, leaf: Leaf::Bits, two_state: false, class: NONE, nested: NONE },
                    Type::Str => Elem { w: 64, signed: false, leaf: Leaf::Str, two_state: true, class: NONE, nested: NONE },
                    Type::Real => Elem { w: 64, signed: false, leaf: Leaf::Real, two_state: true, class: NONE, nested: NONE },
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
        self.method_arr_done(&a, fixed);
        Ok(q)
    }

    /// `arr.sort with (e);` / `arr.rsort with (e);`: false if `e` is no ordering method
    /// with `with`.
    pub(crate) fn arr_order_with(&mut self, e: &Expr, sid: usize) -> R<bool> {
        let ExprKind::ArrayWith { arr, method: m, depth, with: we } = &e.kind else { return Ok(false) };
        if !ORDER.contains(&m.as_str()) {
            return Ok(false);
        }
        if !m.ends_with("sort") {
            return self.err(format!("'{m}' takes no `with` expression"));
        }
        if self.method_arr_type(arr, sid)?.is_none() {
            return self.err(format!("array method '{m}' is not supported for this array (only for queues, dynamic arrays and one-dimensional fixed-size arrays of vectors, reals, strings or class handles)"));
        }
        let (a, fixed) = self.method_arr(arr, sid)?;
        if matches!(a.kind, DynKind::Assoc(_)) {
            return self.err(format!("'{m}' is not a method of an associative array"));
        }
        let (isid, it, ix) = self.iter_scope(we, *depth, a.elem, arr, sid)?;
        let kt = self.type_of(we, isid)?;
        let key = match kt {
            Type::Vec { w, signed } => Elem { w, signed, leaf: Leaf::Bits, two_state: false, class: NONE, nested: NONE },
            Type::Str => Elem { w: 64, signed: false, leaf: Leaf::Str, two_state: true, class: NONE, nested: NONE },
            Type::Real => Elem { w: 64, signed: false, leaf: Leaf::Real, two_state: true, class: NONE, nested: NONE },
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
        match fixed {
            Some(f) => self.queue_to_fixed(a.slot, &f, StoreMode::Blocking, true),
            None => self.dyn_done(&a),
        }
        Ok(true)
    }

    /// `arr.sort` and the other ordering methods as a statement: true if `m` is one.
    pub(crate) fn arr_order(&mut self, a: &DynArr, m: &str, args: &[Expr]) -> R<bool> {
        if !ORDER.contains(&m) {
            return Ok(false);
        }
        if !args.is_empty() && !matches!(args, [Expr { kind: ExprKind::EmptyArg, .. }]) {
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

    /// `e` as a whole one-dimensional fixed-size array (None for anything else, also arrays
    /// of structs).
    pub(crate) fn fixed_arr(&mut self, e: &Expr, sid: usize) -> R<Option<FixedArr>> {
        let sym = match &e.kind {
            ExprKind::Ident(n) => self.lookup(n, sid),
            ExprKind::Hier(p) => self.lookup_hier(p, sid),
            _ => return Ok(None),
        };
        let Some(Sym::Sig(s)) = sym else { return Ok(None) };
        let i = self.sig(s);
        if i.depth == 0 || self.sigs.md_dims.contains_key(&s) || self.sigs.layout.get(&s).is_some_and(|l| l.fields.is_some()) {
            return Ok(None);
        }
        let leaf = if i.real {
            Leaf::Real
        } else if i.string {
            Leaf::Str
        } else {
            Leaf::Bits
        };
        let class = match self.sig_type(s) {
            Type::Class(c) => c,
            _ => NONE,
        };
        let two_state = self.sigs.two_state.contains(&self.canon(s));
        let elem = Elem { w: i.width, signed: i.signed, leaf, two_state, class, nested: NONE };
        Ok(Some(FixedArr { sig: s, n: i.depth, desc: self.sigs.desc_arrays.contains(&s), elem }))
    }

    /// `base.m` / `base.m(args)` where `base` is a fixed-size array and `m` an array
    /// manipulation method: (base, method, arguments).
    fn fixed_method_expr(&mut self, e: &Expr, sid: usize) -> R<Option<(Expr, String, Vec<Expr>)>> {
        let path = |p: &[String]| Expr::from(if p.len() == 1 { ExprKind::Ident(p[0].clone()) } else { ExprKind::Hier(p.to_vec()) });
        let (base, m, args) = match &e.kind {
            ExprKind::Hier(p) if p.len() > 1 => (path(&p[..p.len() - 1]), p[p.len() - 1].clone(), vec![]),
            ExprKind::Call(name, args) => {
                let Some((b, m)) = name.rsplit_once('.') else { return Ok(None) };
                let parts: Vec<String> = b.split('.').map(String::from).collect();
                (path(&parts), m.to_string(), args.clone())
            }
            ExprKind::MethodCall(o, m, args) => ((**o).clone(), m.clone(), args.clone()),
            ExprKind::Member(o, m) => ((**o).clone(), m.clone(), vec![]),
            _ => return Ok(None),
        };
        if ![REDUCE, LOCATE, ORDER].iter().any(|l| l.contains(&m.as_str())) || self.fixed_arr(&base, sid)?.is_none() {
            return Ok(None);
        }
        Ok(Some((base, m, args)))
    }

    /// The kind and element type of the array a method is called on: a queue, dynamic or
    /// associative array, or a fixed-size array (as a dynamic array).
    fn method_arr_type(&mut self, e: &Expr, sid: usize) -> R<Option<(DynKind, Elem)>> {
        if let Some(t) = self.dyn_arr_type(e, sid)? {
            return Ok(Some(t));
        }
        Ok(self.fixed_arr(e, sid)?.map(|f| (DynKind::Darray, f.elem)))
    }

    /// The array a method is called on: a fixed-size array as a temporary copy (and the
    /// array itself).
    fn method_arr(&mut self, e: &Expr, sid: usize) -> R<(DynArr, Option<FixedArr>)> {
        if let Some(a) = self.dyn_arr(e, sid)? {
            return Ok((a, None));
        }
        let f = self.fixed_arr(e, sid)?.unwrap();
        let q = self.fixed_to_queue(&f);
        Ok((DynArr::temp(q, f.elem, self.fixed_left(&f), f.desc), Some(f)))
    }

    /// After a method read the array of `method_arr`: a temporary copy goes.
    fn method_arr_done(&mut self, a: &DynArr, fixed: Option<FixedArr>) {
        if fixed.is_some() {
            self.free_queue(a.slot, a.elem);
        }
    }

    fn free_queue(&mut self, q: u32, elem: Elem) {
        let t = Self::heap_ty_of(elem, DynKind::Queue(0));
        self.emit(Instr::Heap { op: HeapOp::Free, d: NONE, a: q, b: NONE, c: NONE, w: elem.w, t, at: 0 });
    }

    /// A loop over the elements of `f` from left to right: `body` gets the index of each.
    fn fixed_loop(&mut self, f: &FixedArr, mut body: impl FnMut(&mut Self, rt::Index, u32) -> R<()>) -> R<()> {
        let i = self.tmp(32);
        let zero = self.const_slot(&Const::zero(32));
        self.emit(Instr::Copy { d: i, s: zero, w: 32 });
        let n = self.const_slot(&Const::from_i64(f.n as i64, 32, true));
        let top = self.here();
        let c = self.tmp(1);
        self.emit(Instr::Cmp { op: rt::CmpOp::LtS, d: c, a: i, b: n, w: 32 });
        let j = self.emit(Instr::Jz { c, w: 1, to: 0 });
        // (element offset: i from the left, the highest index first for a descending range)
        let ix = rt::Index { slot: i, w: 32, signed: true, neg: f.desc, k: if f.desc { f.n as i64 - 1 } else { 0 } };
        body(self, ix, i)?;
        let one = self.const_slot(&Const::from_i64(1, 32, true));
        self.emit(Instr::Bin { op: rt::BinOp::Add, d: i, a: i, b: one, w: 32 });
        self.emit(Instr::Jmp { to: top });
        let end = self.here();
        self.patch(j, end);
        Ok(())
    }

    /// The index of the leftmost element of `f`.
    fn fixed_left(&self, f: &FixedArr) -> i64 {
        let lo = self.sig(f.sig).arr_lo;
        if f.desc { lo + f.n as i64 - 1 } else { lo }
    }

    /// A new queue with the elements of `f` from left to right: the slot of its handle.
    pub(crate) fn fixed_to_queue(&mut self, f: &FixedArr) -> u32 {
        let q = self.empty_queue();
        let (sig, elem) = (f.sig, f.elem);
        let el = self.tmp(elem.w);
        self.fixed_loop(f, |c, ix, _| {
            c.emit(Instr::ArrRead { d: el, sig, ix });
            c.push(q, el, elem);
            Ok(())
        })
        .unwrap();
        q
    }

    /// The elements of the queue in slot `q` stored into `f` from left to right (`mode`:
    /// blocking or nonblocking); with a different number of elements, an error at run
    /// time and `f` unchanged. `free`: the queue goes afterwards.
    fn queue_to_fixed(&mut self, q: u32, f: &FixedArr, mode: StoreMode, free: bool) {
        let elem = f.elem;
        let t = Self::heap_ty_of(elem, DynKind::Queue(0));
        let ok = self.tmp(1);
        let n = self.const_slot(&Const::from_i64(f.n as i64, 32, true));
        let at = self.loc_id();
        self.emit(Instr::Heap { op: HeapOp::SizeIs, d: ok, a: q, b: NONE, c: n, w: elem.w, t, at });
        let j = self.emit(Instr::Jz { c: ok, w: 1, to: 0 });
        let sig = f.sig;
        let v = self.tmp(elem.w);
        self.fixed_loop(f, |c, ix, i| {
            c.emit(Instr::Heap { op: HeapOp::Get, d: v, a: q, b: i, c: NONE, w: elem.w, t, at: 0 });
            c.emit(Instr::Store { t: rt::Target::Elem { sig, ix }, s: v, mode: mode.clone() });
            Ok(())
        })
        .unwrap();
        let h = self.here();
        self.patch(j, h);
        if free {
            self.free_queue(q, elem);
        }
    }

    /// `f = q` / `f <= q` of a fixed-size array and a queue or dynamic array value: false
    /// if the assignment is no such one.
    pub(crate) fn fixed_from_dyn(&mut self, lhs: &Expr, rhs: &Expr, nonblocking: bool, sid: usize) -> R<bool> {
        let Some(f) = self.fixed_arr(lhs, sid)? else { return Ok(false) };
        let dynamic = self.dyn_arr_type(rhs, sid)?.is_some()
            || match self.arr_method(rhs, sid)? {
                Some(am) => LOCATE.contains(&am.m.as_str()),
                None => false,
            };
        if !dynamic {
            return Ok(false);
        }
        let (q, fresh) = self.dyn_rhs_k(rhs, f.elem, false, sid)?;
        let mode = if nonblocking { StoreMode::Nba } else { StoreMode::Blocking };
        self.queue_to_fixed(q, &f, mode, fresh);
        Ok(true)
    }

    /// `f.sort;` and the other ordering methods of a fixed-size array `base` as a
    /// statement: false if this is no such call.
    pub(crate) fn fixed_order(&mut self, base: &Expr, m: &str, args: &[Expr], sid: usize) -> R<bool> {
        if !ORDER.contains(&m) {
            return Ok(false);
        }
        let Some(f) = self.fixed_arr(base, sid)? else { return Ok(false) };
        let q = self.fixed_to_queue(&f);
        let a = DynArr::temp(q, f.elem, self.fixed_left(&f), f.desc);
        self.arr_order(&a, m, args)?;
        self.queue_to_fixed(a.slot, &f, StoreMode::Blocking, true);
        Ok(true)
    }

    /// A queue, dynamic array or one-dimensional fixed-size array as a display argument (an
    /// assignment pattern, `%p`); None for anything else.
    pub(crate) fn fmt_aggregate(&mut self, e: &Expr, sid: usize) -> R<Option<rt::FmtArg>> {
        let kind_of = |el: Elem| match el.leaf {
            Leaf::Bits => rt::ElemKind::Vec,
            Leaf::Real => rt::ElemKind::Real,
            Leaf::Str => rt::ElemKind::Str,
        };
        // a locator method's queue
        if let Some(am) = self.arr_method(e, sid)? {
            if !LOCATE.contains(&am.m.as_str()) {
                return Ok(None);
            }
            let Type::Dyn { elem, .. } = self.arr_method_type(&am, sid)? else { unreachable!() };
            if elem.class != NONE {
                return self.err("printing an array of class handles is not supported");
            }
            let s = self.arr_locate(&am, sid)?;
            let t = Self::heap_ty_of(elem, DynKind::Queue(0));
            return Ok(Some(rt::FmtArg::Agg { s, w: elem.w, t, signed: elem.signed }));
        }
        if let Some((kind, elem)) = self.dyn_arr_type(e, sid)? {
            if matches!(&e.kind, ExprKind::Index(..) | ExprKind::Range(..) | ExprKind::IndexedRange(..)) {
                return Ok(None);
            }
            if matches!(kind, DynKind::Assoc(_)) {
                return self.err("printing an associative array is not supported");
            }
            if elem.class != NONE {
                return self.err("printing an array of class handles is not supported");
            }
            let a = self.dyn_arr(e, sid)?.unwrap();
            let t = self.heap_ty(&a);
            return Ok(Some(rt::FmtArg::Agg { s: a.slot, w: elem.w, t, signed: elem.signed }));
        }
        if let Some(f) = self.fixed_arr(e, sid)? {
            if f.elem.class != NONE {
                return self.err("printing an array of class handles is not supported");
            }
            return Ok(Some(rt::FmtArg::ArrAgg { sig: f.sig, desc: f.desc, signed: f.elem.signed, elem: kind_of(f.elem) }));
        }
        Ok(None)
    }
}
