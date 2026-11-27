//! `randomize()` (IEEE 1800-2017 18): code at the call that gives the random variables
//! values satisfying the constraints.
//!
//! An attempt gives the random variables values one after the other, each from what simple
//! constraints on it say: `v op e` (`<`, `<=`, `>`, `>=`, `==`) narrows its range,
//! `v inside {...}` and `v dist {...}` pick from the set (a dist by its weights); a
//! constraint under an implication or `if` counts when its condition holds. Such a rule is
//! used when the random variables in `e` and in the conditions have their values already;
//! the variables are ordered so that this holds as far as it can. Then all constraints are
//! checked; an attempt that fails is repeated, first with the soft constraints, then without
//! them. Without a solution the variables keep their values, `randomize()` gives 0 and a
//! warning is printed. The values are valid solutions, not uniformly distributed among
//! them.

use crate::*;
use crate::types::{DynKind, Type};
use ferrooxide_front::ast::{BinOp, CItem, UnOp};
use rt::{ObjOp, SysFn, Target};

/// Attempts per phase (with soft constraints, without them).
const TRIES: i64 = 1000;

/// A random variable: how constraints name it (`this.p`, `obj.p`, `this.a[2]`, `x`) and where
/// its value lives.
#[derive(Clone)]
struct RVar {
    names: Vec<String>,
    w: u32,
    signed: bool,
    home: Home,
    /// a `randc` variable (IEEE 1800-2017 18.4.2): the key of its cycle's state
    cyc: Option<u32>,
}

#[derive(Clone)]
enum Home {
    /// bits at `off` of the object
    Prop { off: u32 },
    /// a variable (`std::randomize(x)`) in scope `sid`
    Var { e: Expr, sid: usize },
}

/// A rand dynamic array or queue property.
struct RDyn {
    name: String,
    /// the scope of the class that declares it
    sid: usize,
    queue: bool,
    /// of handles: their objects are randomized
    objs: bool,
    /// the signal holding its new size (a constraint names its size)
    size: Option<u32>,
}

/// A constraint item with the scope its expressions are compiled in.
#[derive(Clone)]
struct Item {
    it: CItem,
    sid: usize,
}

/// What a constraint says about one variable, under conditions (expression, value).
struct Rule<'r> {
    guards: Vec<(&'r Expr, bool, usize)>,
    kind: RuleKind<'r>,
    soft: bool,
    sid: usize,
    /// the other random variables it reads (indexes into the variables)
    deps: Vec<usize>,
}

enum RuleKind<'r> {
    /// `v > e` (strict) / `v >= e`
    Lo(&'r Expr, bool),
    /// `v < e` / `v <= e`
    Hi(&'r Expr, bool),
    Eq(&'r Expr),
    Inside(&'r [(Expr, Option<Expr>)]),
    Dist(&'r [ferrooxide_front::ast::DistItem]),
}

/// A key for comparing expressions.
fn key(e: &Expr) -> String {
    format!("{e:?}")
}

fn num(v: i64) -> Expr {
    let n = ExprKind::Num(ferrooxide_front::ast::dec(v.unsigned_abs()));
    if v < 0 { ExprKind::Unary(UnOp::Neg, Box::new(n.into())).into() } else { n.into() }
}

fn bin(op: BinOp, a: Expr, b: Expr) -> Expr {
    ExprKind::Binary(op, Box::new(a), Box::new(b)).into()
}

/// `e` with identifier `name` replaced by `v` (a `foreach` variable by its index).
fn subst(e: &Expr, name: &str, v: i64) -> Expr {
    let s = |x: &Expr| Box::new(subst(x, name, v));
    let k = match &e.kind {
        ExprKind::Ident(n) if n == name => return num(v),
        ExprKind::Index(a, b) => ExprKind::Index(s(a), s(b)),
        ExprKind::Range(a, b, c) => ExprKind::Range(s(a), s(b), s(c)),
        ExprKind::IndexedRange(a, b, c, up) => ExprKind::IndexedRange(s(a), s(b), s(c), *up),
        ExprKind::Cond(a, b, c) => ExprKind::Cond(s(a), s(b), s(c)),
        ExprKind::Binary(op, a, b) => ExprKind::Binary(*op, s(a), s(b)),
        ExprKind::Unary(op, a) => ExprKind::Unary(*op, s(a)),
        ExprKind::Member(a, m) => ExprKind::Member(s(a), m.clone()),
        ExprKind::Concat(v2) => ExprKind::Concat(v2.iter().map(|x| subst(x, name, v)).collect()),
        ExprKind::Call(f, v2) => ExprKind::Call(f.clone(), v2.iter().map(|x| subst(x, name, v)).collect()),
        ExprKind::SysCall(f, v2) => ExprKind::SysCall(f.clone(), v2.iter().map(|x| subst(x, name, v)).collect()),
        ExprKind::MethodCall(o, m, v2) => ExprKind::MethodCall(s(o), m.clone(), v2.iter().map(|x| subst(x, name, v)).collect()),
        ExprKind::Repl(n, v2) => ExprKind::Repl(s(n), v2.iter().map(|x| subst(x, name, v)).collect()),
        _ => return e.clone(),
    };
    Expr::new(k, e.line)
}

fn subst_item(it: &CItem, name: &str, v: i64) -> CItem {
    let e = |x: &Expr| subst(x, name, v);
    let items = |x: &[CItem]| x.iter().map(|i| subst_item(i, name, v)).collect::<Vec<_>>();
    match it {
        CItem::Expr { soft, e: x } => CItem::Expr { soft: *soft, e: e(x) },
        CItem::Inside { soft, e: x, set } => CItem::Inside { soft: *soft, e: e(x), set: set.iter().map(|(a, b)| (e(a), b.as_ref().map(e))).collect() },
        CItem::Dist { soft, e: x, items: d } => CItem::Dist {
            soft: *soft,
            e: e(x),
            items: d.iter().map(|i| ferrooxide_front::ast::DistItem { lo: e(&i.lo), hi: i.hi.as_ref().map(e), per_value: i.per_value, weight: e(&i.weight) }).collect(),
        },
        CItem::Implies(g, v2) => CItem::Implies(e(g), items(v2)),
        CItem::If(g, a, b) => CItem::If(e(g), items(a), items(b)),
        CItem::Foreach { arr, vars, items: v2 } => {
            // (an inner loop's own variable of that name hides it)
            if vars.iter().flatten().any(|x| x == name) {
                return it.clone();
            }
            CItem::Foreach { arr: e(arr), vars: vars.clone(), items: items(v2) }
        }
        CItem::Unique(v2) => CItem::Unique(v2.iter().map(e).collect()),
        CItem::SolveBefore => CItem::SolveBefore,
        CItem::DisableSoft(x) => CItem::DisableSoft(e(x)),
    }
}

/// `e` with each subexpression whose key is one of `keys` replaced by `to`.
fn replace(e: &Expr, keys: &[String], to: &Expr) -> Expr {
    if keys.contains(&key(e)) {
        return to.clone();
    }
    let mut e = e.clone();
    e.for_each_child_mut(&mut |c| *c = replace(c, keys, to));
    e
}

/// The item with `f` applied to each of its expressions.
fn map_item(it: &CItem, f: &dyn Fn(&Expr) -> Expr) -> CItem {
    let items = |x: &[CItem]| x.iter().map(|i| map_item(i, f)).collect::<Vec<_>>();
    match it {
        CItem::Expr { soft, e } => CItem::Expr { soft: *soft, e: f(e) },
        CItem::Inside { soft, e, set } => CItem::Inside { soft: *soft, e: f(e), set: set.iter().map(|(a, b)| (f(a), b.as_ref().map(f))).collect() },
        CItem::Dist { soft, e, items: d } => CItem::Dist {
            soft: *soft,
            e: f(e),
            items: d.iter().map(|i| ferrooxide_front::ast::DistItem { lo: f(&i.lo), hi: i.hi.as_ref().map(f), per_value: i.per_value, weight: f(&i.weight) }).collect(),
        },
        CItem::Implies(g, v) => CItem::Implies(f(g), items(v)),
        CItem::If(g, a, b) => CItem::If(f(g), items(a), items(b)),
        CItem::Foreach { arr, vars, items: v } => CItem::Foreach { arr: f(arr), vars: vars.clone(), items: items(v) },
        CItem::Unique(v) => CItem::Unique(v.iter().map(f).collect()),
        CItem::SolveBefore => CItem::SolveBefore,
        CItem::DisableSoft(x) => CItem::DisableSoft(f(x)),
    }
}

impl<'a> Compiler<'a> {
    /// `randomize()` (`ExprKind::Randomize`): the slot of its 32-bit result.
    pub(crate) fn randomize(&mut self, e: &Expr, sid: usize) -> R<u32> {
        let ExprKind::Randomize { obj, vars, with, std } = &e.kind else { unreachable!() };
        let res = self.tmp(32);
        if *std {
            let items: Vec<Item> = with.iter().flatten().map(|it| Item { it: it.clone(), sid }).collect();
            let mut rv = Vec::new();
            for v in vars {
                let (w, signed) = match self.type_of(v, sid)? {
                    Type::Vec { w, signed } => (w, signed),
                    _ => return self.err("std::randomize() of a variable that is not integral is not supported"),
                };
                rv.push(RVar { names: vec![key(v)], w, signed, home: Home::Var { e: v.clone(), sid }, cyc: None });
            }
            let at = self.loc();
            self.solve(&rv, &items, None, res, &at)?;
            return Ok(res);
        }
        // the object and the class of the handle
        let (h, c) = match obj {
            Some(o) => {
                let Type::Class(c) = self.type_of(o, sid)? else { return self.err("randomize() of something that is not an object") };
                (self.handle_value(o, sid)?, c)
            }
            None => {
                let Some(c) = self.class_of_scope(sid) else { return self.err("randomize() outside a class needs variables to randomize") };
                (self.handle_value(&Expr::from(ExprKind::Ident("this".into())), sid)?, c)
            }
        };
        // the object's own class decides (randomize() is virtual): its variables and
        // constraints
        let classes: Vec<u32> = (0..self.classes.list.len() as u32).filter(|&d| d == c || self.derives(d, c)).collect();
        let cls = self.tmp(32);
        self.emit(Instr::Obj { op: ObjOp::Class, d: cls, a: h, b: NONE, off: 0, w: 0 });
        let zero = self.const_slot(&Const::zero(32));
        self.emit(Instr::Copy { d: res, s: zero, w: 32 });
        let mut ends = Vec::new();
        for d in classes {
            if self.classes.list[d as usize].decl.abstract_ {
                continue;
            }
            let k = self.const_slot(&Const::from_i64(d as i64, 32, false));
            let is = self.tmp(1);
            self.emit(Instr::Cmp { op: rt::CmpOp::Eq, d: is, a: cls, b: k, w: 32 });
            let skip = self.emit(Instr::Jz { c: is, w: 1, to: 0 });
            self.randomize_class(d, h, obj.as_deref(), vars, with.as_deref(), sid, res)?;
            ends.push(self.emit(Instr::Jmp { to: 0 }));
            let here = self.here();
            self.patch(skip, here);
        }
        let end = self.here();
        for j in ends {
            self.patch(j, end);
        }
        Ok(res)
    }

    /// The classes of `d` from its first base class to itself.
    fn class_chain(&self, d: u32) -> Vec<u32> {
        let mut v = vec![d];
        let mut c = d;
        while let Some(b) = self.classes.list[c as usize].base {
            v.push(b);
            c = b;
        }
        v.reverse();
        v
    }

    /// `randomize()` of an object of class `d` (handle in slot `h`).
    #[allow(clippy::too_many_arguments)]
    fn randomize_class(&mut self, d: u32, h: u32, obj: Option<&Expr>, vars: &[Expr], with: Option<&[CItem]>, sid: usize, res: u32) -> R<()> {
        let chain = self.class_chain(d);
        // `this` of each class's constraints
        for &c in &chain {
            let t = self.classes.list[c as usize].init_this;
            let ts = self.sig(t).slot;
            self.emit(Instr::Copy { d: ts, s: h, w: 64 });
        }
        // the random variables: the rand properties, or those `randomize(v)` names
        // (with the scope of the class that declares each, for a static one)
        let mut names: Vec<(String, bool)> = Vec::new();
        let mut owner_scope: Vec<usize> = Vec::new();
        if vars.is_empty() {
            for &c in &chain {
                let ci = &self.classes.list[c as usize];
                names.extend(ci.decl.rand.iter().cloned());
                owner_scope.resize(names.len(), ci.init_scope);
            }
        } else {
            for v in vars {
                match &v.kind {
                    ExprKind::Member(_, p) => {
                        let cyclic = chain.iter().any(|&c| self.classes.list[c as usize].decl.rand.iter().any(|(n, cy)| *cy && n == p));
                        names.push((p.clone(), cyclic))
                    }
                    _ => return self.err("randomize() of something that is not a property of the object is not supported"),
                }
            }
        }
        let props = self.classes.list[d as usize].props.clone();
        let this_k = |p: &str| key(&Expr::from(ExprKind::Member(Box::new(ExprKind::Ident("this".into()).into()), p.into())));
        let obj_k = |p: &str| obj.map(|o| key(&Expr::from(ExprKind::Member(Box::new(o.clone()), p.into()))));
        let mut rv = Vec::new();
        // rand class handle properties (18.4): their objects are randomized too
        let mut subs: Vec<(String, usize)> = Vec::new();
        let mut dyns: Vec<RDyn> = Vec::new();
        for (k, (n, cyclic)) in names.iter().enumerate() {
            // a static rand property (IEEE 1800-2017 18.4): its variable `C$p`, shared by the
            // objects of the class; constraints name it so
            if let (Some(&psid), Some((_, p))) = (owner_scope.get(k), n.split_once('$')) {
                let e = Expr::from(ExprKind::Ident(n.clone()));
                let (w, signed) = match self.type_of(&e, psid)? {
                    Type::Vec { w, signed } => (w, signed),
                    _ => return self.err(format!("rand property '{p}': only integral properties can be random (others are not supported)")),
                };
                let mut k = vec![key(&e), this_k(p)];
                k.extend(obj_k(p));
                let cyc = match (cyclic, self.lookup(n, psid)) {
                    (false, _) => None,
                    (true, Some(Sym::Sig(id))) => Some(1 << 31 | id),
                    (true, _) => return self.err(format!("randc property '{p}': not a variable")),
                };
                if cyc.is_some() && w > 64 {
                    return self.err(format!("randc property '{p}': a randc variable of more than 64 bits is not supported"));
                }
                rv.push(RVar { names: k, w, signed, home: Home::Var { e, sid: psid }, cyc });
                continue;
            }
            let Some(p) = props.iter().rev().find(|p| &p.name == n) else {
                return self.err(format!("'{n}' is not a property of the object"));
            };
            if let (Type::Class(_), []) = (&p.ty, p.dims.as_slice()) {
                let owner = chain.iter().rev().find(|&&c| self.classes.list[c as usize].props.iter().any(|q| q.name == *n && q.off == p.off)).copied().unwrap_or(d);
                subs.push((n.clone(), self.classes.list[owner as usize].init_scope));
                continue;
            }
            // a rand dynamic array or queue (18.4): its elements are random, its size when
            // a constraint names `size()`; one of handles: the objects are randomized
            if let (Type::Dyn { kind: kind @ (DynKind::Darray | DynKind::Queue(_)), elem }, []) = (&p.ty, p.dims.as_slice()) {
                let owner = chain.iter().rev().find(|&&c| self.classes.list[c as usize].props.iter().any(|q| q.name == *n && q.off == p.off)).copied().unwrap_or(d);
                let osid = self.classes.list[owner as usize].init_scope;
                if *cyclic {
                    return self.err(format!("randc property '{n}': a randc array of variable size is not supported"));
                }
                if elem.nested != NONE || (elem.class == NONE && elem.leaf != crate::types::Leaf::Bits) {
                    return self.err(format!("rand property '{n}': an array of this element type is not supported as random"));
                }
                dyns.push(RDyn { name: n.clone(), sid: osid, queue: matches!(kind, DynKind::Queue(_)), objs: elem.class != NONE, size: None });
                continue;
            }
            let (w, signed) = match p.ty {
                Type::Vec { w, signed } => (w, signed),
                _ => return self.err(format!("rand property '{n}': only integral properties can be random (others are not supported)")),
            };
            if *cyclic && w > 64 {
                return self.err(format!("randc property '{n}': a randc variable of more than 64 bits is not supported"));
            }
            let cyc = |off: u32| cyclic.then_some(off);
            match p.dims.as_slice() {
                [] => {
                    let mut k = vec![this_k(n)];
                    k.extend(obj_k(n));
                    rv.push(RVar { names: k, w, signed, home: Home::Prop { off: p.off }, cyc: cyc(p.off) });
                }
                [(lo, cnt)] => {
                    for e in 0..*cnt {
                        let ix = |b: Expr| key(&Expr::from(ExprKind::Index(Box::new(b), Box::new(num(*lo + e as i64)))));
                        let mut k = vec![ix(ExprKind::Member(Box::new(ExprKind::Ident("this".into()).into()), n.clone()).into())];
                        if let Some(o) = obj {
                            k.push(ix(ExprKind::Member(Box::new(o.clone()), n.clone()).into()));
                        }
                        rv.push(RVar { names: k, w, signed, home: Home::Prop { off: p.off + e * w }, cyc: cyc(p.off + e * w) });
                    }
                }
                _ => return self.err(format!("rand property '{n}': a multi-dimensional rand array is not supported")),
            }
        }
        // the constraints: of the class and its bases (one of a base class is replaced by
        // the derived class's of the same name), and those of `with`
        let mut items = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        for &c in chain.iter().rev() {
            let ci = &self.classes.list[c as usize];
            for k in &ci.decl.constraints {
                if seen.contains(&k.name.as_str()) {
                    continue;
                }
                seen.push(&k.name);
                items.extend(k.items.iter().map(|it| Item { it: it.clone(), sid: ci.init_scope }));
            }
        }
        items.extend(with.iter().flat_map(|w| w.iter()).map(|it| Item { it: it.clone(), sid }));
        // (the objects of rand handles are randomized on their own: constraints that name
        // their members are not supported)
        for (n, _) in &subs {
            let is_sub = |x: &Expr| -> bool {
                let ExprKind::Member(o, _) = &x.kind else { return false };
                match &o.kind {
                    ExprKind::Ident(i) => i == n,
                    ExprKind::Member(t, i) => i == n && (matches!(&t.kind, ExprKind::Ident(th) if th == "this") || obj.is_some_and(|ob| key(ob) == key(t))),
                    _ => false,
                }
            };
            let names_sub = |e: &Expr| Self::any_expr(e, &is_sub);
            if items.iter().any(|i| names_sub(&Self::item_expr(&i.it))) {
                return self.err(format!("a constraint on members of the rand object property '{n}' is not supported"));
            }
        }
        // the random arrays: constraints may name their sizes only (one on their elements is
        // not supported); a size is one more variable, at least 0
        let this_e = |n: &str| Expr::from(ExprKind::Member(Box::new(ExprKind::Ident("this".into()).into()), n.into()));
        let size_e = |a: Expr| Expr::from(ExprKind::MethodCall(Box::new(a), "size".into(), vec![]));
        for dy in dyns.iter_mut() {
            let mut arr = vec![key(&this_e(&dy.name))];
            let mut sizes = vec![key(&size_e(this_e(&dy.name)))];
            if let Some(o) = obj {
                let m = Expr::from(ExprKind::Member(Box::new(o.clone()), dy.name.clone()));
                arr.push(key(&m));
                sizes.push(key(&size_e(m)));
            }
            let (mut sized, mut elems) = (false, false);
            for i in &items {
                Self::size_uses(&Self::item_expr(&i.it), &arr, &sizes, &mut sized, &mut elems);
                if let CItem::Foreach { arr: a, .. } = &i.it {
                    elems |= arr.contains(&key(a));
                }
            }
            if elems {
                return self.err(format!("a constraint on the elements of the rand array '{}' is not supported (only on its size)", dy.name));
            }
            if !sized {
                continue;
            }
            if dy.queue {
                return self.err(format!("a constraint on the size of the rand queue '{}' is not supported", dy.name));
            }
            let (hn, hs) = self.rand_hidden(dy.sid, 32);
            // (the constraints of `with` name it in the caller's scope)
            self.scopes[sid].syms.insert(hn.clone(), Sym::Sig(hs));
            // (it starts at the array's size: kept when there is no solution)
            let st = Stmt::Assign { lhs: ExprKind::Ident(hn.clone()).into(), rhs: size_e(this_e(&dy.name)), nonblocking: false, timing: None, line: self.line };
            self.stmt(&st, dy.sid)?;
            let he = Expr::from(ExprKind::Ident(hn.clone()));
            // (the constraints name the new size: the variable)
            for i in items.iter_mut() {
                i.it = map_item(&i.it, &|e| replace(e, &sizes, &he));
            }
            rv.push(RVar { names: vec![key(&he)], w: 32, signed: true, home: Home::Var { e: he.clone(), sid: dy.sid }, cyc: None });
            items.push(Item { it: CItem::Expr { soft: false, e: bin(BinOp::Ge, he, num(0)) }, sid: dy.sid });
            dy.size = Some(hs);
        }
        // pre_randomize(), then the values, then post_randomize() after a success
        let at = self.loc();
        if let Some((f, owner)) = self.find_method(d, "pre_randomize") {
            let scope = self.class_scope(owner);
            self.call_resolved(f, scope, "pre_randomize", Some(h), &[], sid)?;
        }
        let outer = self.code.rand_obj.replace(h);
        let r = self.solve(&rv, &items, Some(h), res, &at);
        let r = r.and_then(|_| self.rand_arrays(&dyns, d, res));
        self.code.rand_obj = outer;
        r?;
        // the objects of the rand handles that are not null, after a success
        if !subs.is_empty() {
            if self.code.rand_nest.contains(&d) {
                return self.err(format!("randomize() of class '{}' through a rand property of its own class is not supported", self.classes.list[d as usize].name));
            }
            self.code.rand_nest.push(d);
            let r = (|| -> R<()> {
                for (n, psid) in &subs {
                    let skip = self.emit(Instr::Jz { c: res, w: 32, to: 0 });
                    let m = Expr::from(ExprKind::Member(Box::new(ExprKind::Ident("this".into()).into()), n.clone()));
                    // (`this` of the class's constraints is the object here)
                    let ps = self.sig(self.classes.list[self.class_of_scope(*psid).unwrap() as usize].init_this).slot;
                    self.emit(Instr::Copy { d: ps, s: h, w: 64 });
                    let sh = self.handle_value(&m, *psid)?;
                    let nn = self.tmp(1);
                    self.emit(Instr::Red { op: rt::RedOp::Bool, d: nn, s: sh, w: 64 });
                    let null = self.emit(Instr::Jz { c: nn, w: 1, to: 0 });
                    let call = Expr::from(ExprKind::Randomize { obj: Some(Box::new(m)), vars: vec![], with: None, std: false });
                    let r2 = self.randomize(&call, *psid)?;
                    self.emit(Instr::Copy { d: res, s: r2, w: 32 });
                    self.patch_all(vec![skip, null]);
                }
                Ok(())
            })();
            self.code.rand_nest.pop();
            r?;
        }
        if let Some((f, owner)) = self.find_method(d, "post_randomize") {
            let skip = self.emit(Instr::Jz { c: res, w: 32, to: 0 });
            let scope = self.class_scope(owner);
            self.call_resolved(f, scope, "post_randomize", Some(h), &[], sid)?;
            let here = self.here();
            self.patch(skip, here);
        }
        Ok(())
    }

    /// Whether `e` names the size of an array (one of the keys `sizes`) or the array (`arr`)
    /// otherwise.
    fn size_uses(e: &Expr, arr: &[String], sizes: &[String], sized: &mut bool, elems: &mut bool) {
        let k = key(e);
        if sizes.contains(&k) {
            *sized = true;
            return;
        }
        if arr.contains(&k) {
            *elems = true;
            return;
        }
        let mut e2 = e.clone();
        e2.for_each_child_mut(&mut |c| Self::size_uses(c, arr, sizes, sized, elems));
    }

    /// A variable of the compiler's own (`$randN`, `w` bits, signed) in scope `sid`: its name
    /// and signal.
    fn rand_hidden(&mut self, sid: usize, w: u32) -> (String, u32) {
        self.subr.auto_count += 1;
        let n = format!("$rand{}", self.subr.auto_count);
        let path = format!("{}.{n}", self.scopes[sid].name);
        let s = self.new_signal(path, w, true, 0, w as i64 - 1, 0, 0, SigKind::Reg);
        self.scopes[sid].syms.insert(n.clone(), Sym::Sig(s));
        (n, s)
    }

    /// After a successful attempt (`res` 1) of class `d`: the random arrays get their new
    /// sizes and random elements; the objects of arrays of handles that are not null are
    /// randomized (`res` 0 when one fails).
    fn rand_arrays(&mut self, dyns: &[RDyn], d: u32, res: u32) -> R<()> {
        if dyns.is_empty() {
            return Ok(());
        }
        let skip = self.emit(Instr::Jz { c: res, w: 32, to: 0 });
        let line = self.line;
        for dy in dyns {
            let a = Expr::from(ExprKind::Member(Box::new(ExprKind::Ident("this".into()).into()), dy.name.clone()));
            if let Some(hs) = dy.size {
                let hn = self.sig(hs).name.rsplit('.').next().unwrap().to_string();
                let new = Expr::from(ExprKind::NewArray(Box::new(ExprKind::Ident(hn).into()), None));
                self.stmt(&Stmt::Assign { lhs: a.clone(), rhs: new, nonblocking: false, timing: None, line }, dy.sid)?;
            }
            self.subr.auto_count += 1;
            let iv = format!("$ri{}", self.subr.auto_count);
            let el = Expr::from(ExprKind::Index(Box::new(a.clone()), Box::new(ExprKind::Ident(iv.clone()).into())));
            let body = if dy.objs {
                if self.code.rand_nest.contains(&d) {
                    return self.err(format!("randomize() of class '{}' through a rand property of its own class is not supported", self.classes.list[d as usize].name));
                }
                // if (a[i] != null && !a[i].randomize()) fail = 1
                let (fname, fs) = self.rand_hidden(dy.sid, 32);
                let zero = self.const_slot(&Const::zero(32));
                let fslot = self.sig(fs).slot;
                self.emit(Instr::Store { t: Target::Whole { sig: fs }, s: zero, mode: rt::StoreMode::Blocking });
                let call = Expr::from(ExprKind::Randomize { obj: Some(Box::new(el.clone())), vars: vec![], with: None, std: false });
                let bad = bin(BinOp::LAnd, bin(BinOp::Ne, el.clone(), ExprKind::Null.into()), ExprKind::Unary(UnOp::LNot, Box::new(call)).into());
                let set = Stmt::Assign { lhs: ExprKind::Ident(fname).into(), rhs: num(1), nonblocking: false, timing: None, line };
                let st = Stmt::If { cond: bad, then_s: Box::new(set), else_s: None, line };
                self.code.rand_nest.push(d);
                let r = self.stmt(&Stmt::Foreach { array: a, vars: vec![Some(iv)], body: Box::new(st), line }, dy.sid);
                self.code.rand_nest.pop();
                r?;
                let fv = self.tmp(32);
                self.emit(Instr::Copy { d: fv, s: fslot, w: 32 });
                let ok = self.tmp(1);
                self.emit(Instr::Cmp { op: rt::CmpOp::Eq, d: ok, a: fv, b: zero, w: 32 });
                let okw = self.tmp(32);
                self.emit(Instr::Copy { d: okw, s: zero, w: 32 });
                self.emit(Instr::Copy { d: okw, s: ok, w: 1 });
                self.emit(Instr::Copy { d: res, s: okw, w: 32 });
                continue;
            } else {
                // (from the object's random generator)
                Stmt::Discard { e: ExprKind::Randomize { obj: None, vars: vec![el], with: None, std: true }.into(), line }
            };
            self.stmt(&Stmt::Foreach { array: a, vars: vec![Some(iv)], body: Box::new(body), line }, dy.sid)?;
        }
        let here = self.here();
        self.patch(skip, here);
        Ok(())
    }

    /// The items with `foreach` unrolled over the fixed-size arrays it names.
    fn unroll(&mut self, items: &[Item]) -> R<Vec<Item>> {
        let mut out = Vec::new();
        for i in items {
            match &i.it {
                CItem::Foreach { arr, vars, items: body } => {
                    let Some(var) = vars.first().cloned().flatten() else {
                        return self.err("foreach without a loop variable in a constraint");
                    };
                    if vars.len() > 1 {
                        return self.err("foreach over several dimensions in a constraint is not supported");
                    }
                    let (lo, n) = self.array_bounds(arr, i.sid)?;
                    for k in 0..n as i64 {
                        let b: Vec<Item> = body.iter().map(|x| Item { it: subst_item(x, &var, lo + k), sid: i.sid }).collect();
                        out.extend(self.unroll(&b)?);
                    }
                }
                CItem::Implies(g, v) => {
                    let v = self.unroll(&v.iter().map(|x| Item { it: x.clone(), sid: i.sid }).collect::<Vec<_>>())?;
                    out.push(Item { it: CItem::Implies(g.clone(), v.into_iter().map(|x| x.it).collect()), sid: i.sid });
                }
                CItem::If(g, a, b) => {
                    let a = self.unroll(&a.iter().map(|x| Item { it: x.clone(), sid: i.sid }).collect::<Vec<_>>())?;
                    let b = self.unroll(&b.iter().map(|x| Item { it: x.clone(), sid: i.sid }).collect::<Vec<_>>())?;
                    out.push(Item { it: CItem::If(g.clone(), a.into_iter().map(|x| x.it).collect(), b.into_iter().map(|x| x.it).collect()), sid: i.sid });
                }
                _ => out.push(i.clone()),
            }
        }
        Ok(out)
    }

    /// The lowest index and the number of elements of the one-dimensional fixed-size array
    /// `arr` (a property through a handle, a variable) names.
    fn array_bounds(&mut self, arr: &Expr, sid: usize) -> R<(i64, u32)> {
        if let ExprKind::Member(o, p) = &arr.kind {
            if let Type::Class(c) = self.type_of(o, sid)? {
                if let Some(pr) = self.classes.list[c as usize].props.iter().rev().find(|x| &x.name == p) {
                    if let [(lo, n)] = pr.dims.as_slice() {
                        return Ok((*lo, *n));
                    }
                }
            }
        }
        if let Some(f) = self.fixed_arr(arr, sid)? {
            let lo = self.sig(f.sig).arr_lo;
            return Ok((lo, f.n));
        }
        self.err("foreach in a constraint over something that is not a one-dimensional fixed-size array is not supported")
    }

    /// Whether `e` reads one of the random variables.
    fn reads_rand(e: &Expr, rv: &[RVar]) -> bool {
        let k = key(e);
        if rv.iter().any(|v| v.names.contains(&k)) {
            return true;
        }
        match &e.kind {
            ExprKind::Member(a, _) => {
                // (a whole rand array: its elements' names start with it)
                let m = format!("Index({k}");
                rv.iter().any(|v| v.names.iter().any(|n| n.starts_with(&m))) || Self::reads_rand(a, rv)
            }
            ExprKind::Index(a, b) | ExprKind::Binary(_, a, b) => Self::reads_rand(a, rv) || Self::reads_rand(b, rv),
            ExprKind::Range(a, b, c) | ExprKind::IndexedRange(a, b, c, _) | ExprKind::Cond(a, b, c) => Self::reads_rand(a, rv) || Self::reads_rand(b, rv) || Self::reads_rand(c, rv),
            ExprKind::Unary(_, a) => Self::reads_rand(a, rv),
            ExprKind::Concat(v) | ExprKind::Call(_, v) | ExprKind::SysCall(_, v) | ExprKind::StaticCall(_, _, v) => v.iter().any(|x| Self::reads_rand(x, rv)),
            ExprKind::MethodCall(o, _, v) => Self::reads_rand(o, rv) || v.iter().any(|x| Self::reads_rand(x, rv)),
            ExprKind::Repl(n, v) => Self::reads_rand(n, rv) || v.iter().any(|x| Self::reads_rand(x, rv)),
            _ => false,
        }
    }

    /// Whether `f` holds for `e` or an expression in it.
    fn any_expr(e: &Expr, f: &dyn Fn(&Expr) -> bool) -> bool {
        if f(e) {
            return true;
        }
        let a = |x: &Expr| Self::any_expr(x, f);
        match &e.kind {
            ExprKind::Member(x, _) | ExprKind::Unary(_, x) => a(x),
            ExprKind::Index(x, y) | ExprKind::Binary(_, x, y) => a(x) || a(y),
            ExprKind::Range(x, y, z) | ExprKind::IndexedRange(x, y, z, _) | ExprKind::Cond(x, y, z) => a(x) || a(y) || a(z),
            ExprKind::Concat(v) | ExprKind::Call(_, v) | ExprKind::SysCall(_, v) | ExprKind::StaticCall(_, _, v) => v.iter().any(a),
            ExprKind::MethodCall(o, _, v) | ExprKind::Repl(o, v) => a(o) || v.iter().any(a),
            _ => false,
        }
    }

    /// The random variables (indexes into `rv`) `e` reads.
    fn deps_of(e: &Expr, rv: &[RVar]) -> Vec<usize> {
        (0..rv.len()).filter(|&k| Self::reads_rand(e, std::slice::from_ref(&rv[k]))).collect()
    }

    /// The rules of the items for variable `v` (index into `rv`).
    fn rules<'r>(items: &'r [Item], rv: &[RVar], v: usize, out: &mut Vec<Rule<'r>>) {
        type Guards<'r> = Vec<(&'r Expr, bool, usize)>;
        fn walk<'r>(it: &'r CItem, sid: usize, guards: &mut Guards<'r>, gdeps: &mut Vec<usize>, rv: &[RVar], v: usize, out: &mut Vec<Rule<'r>>) {
            let is_v = |e: &Expr| rv[v].names.contains(&key(e));
            // the variables an expression reads (None: v itself)
            let deps = |es: &[&Expr]| -> Option<Vec<usize>> {
                let mut d = gdeps.clone();
                for e in es {
                    for k in Compiler::deps_of(e, rv) {
                        if k == v {
                            return None;
                        }
                        if !d.contains(&k) {
                            d.push(k);
                        }
                    }
                }
                Some(d)
            };
            let rule = |kind: RuleKind<'r>, soft: bool, guards: &Guards<'r>, deps: Vec<usize>| Rule { guards: guards.clone(), kind, soft, sid, deps };
            match it {
                CItem::Expr { soft, e } => {
                    // (a conjunction: each part)
                    let mut parts = vec![e];
                    while let Some(x) = parts.pop() {
                        let ExprKind::Binary(op, a, b) = &x.kind else { continue };
                        if *op == BinOp::LAnd {
                            parts.push(a);
                            parts.push(b);
                            continue;
                        }
                        // `v op e` or `e op v`
                        let (op, other) = if is_v(a) {
                            (*op, &**b)
                        } else if is_v(b) {
                            let flip = match op {
                                BinOp::Lt => BinOp::Gt,
                                BinOp::Le => BinOp::Ge,
                                BinOp::Gt => BinOp::Lt,
                                BinOp::Ge => BinOp::Le,
                                o => *o,
                            };
                            (flip, &**a)
                        } else {
                            continue;
                        };
                        let kind = match op {
                            BinOp::Lt => RuleKind::Hi(other, true),
                            BinOp::Le => RuleKind::Hi(other, false),
                            BinOp::Gt => RuleKind::Lo(other, true),
                            BinOp::Ge => RuleKind::Lo(other, false),
                            BinOp::Eq => RuleKind::Eq(other),
                            _ => continue,
                        };
                        if let Some(d) = deps(&[other]) {
                            out.push(rule(kind, *soft, guards, d));
                        }
                    }
                }
                CItem::Inside { soft, e, set } if is_v(e) => {
                    let es: Vec<&Expr> = set.iter().flat_map(|(a, b)| std::iter::once(a).chain(b.iter())).collect();
                    if let Some(d) = deps(&es) {
                        out.push(rule(RuleKind::Inside(set), *soft, guards, d));
                    }
                }
                CItem::Dist { soft, e, items } if is_v(e) => {
                    let es: Vec<&Expr> = items.iter().flat_map(|x| std::iter::once(&x.lo).chain(x.hi.iter()).chain(std::iter::once(&x.weight))).collect();
                    if let Some(d) = deps(&es) {
                        out.push(rule(RuleKind::Dist(items), *soft, guards, d));
                    }
                }
                CItem::Implies(g, items) => {
                    let Some(d) = deps(&[g]) else { return };
                    let saved = std::mem::replace(gdeps, d);
                    guards.push((g, true, sid));
                    items.iter().for_each(|x| walk(x, sid, guards, gdeps, rv, v, out));
                    guards.pop();
                    *gdeps = saved;
                }
                CItem::If(g, a, b) => {
                    let Some(d) = deps(&[g]) else { return };
                    let saved = std::mem::replace(gdeps, d);
                    for (pol, branch) in [(true, a), (false, b)] {
                        guards.push((g, pol, sid));
                        branch.iter().for_each(|x| walk(x, sid, guards, gdeps, rv, v, out));
                        guards.pop();
                    }
                    *gdeps = saved;
                }
                _ => {}
            }
        }
        for i in items {
            walk(&i.it, i.sid, &mut Vec::new(), &mut Vec::new(), rv, v, out);
        }
    }

    /// The order in which the variables get values: each after those its rules read where
    /// possible (declaration order otherwise).
    fn rand_order(rules_of: &[Vec<Rule>]) -> Vec<usize> {
        let n = rules_of.len();
        let mut placed: Vec<usize> = Vec::new();
        while placed.len() < n {
            let unmet = |v: usize, placed: &[usize]| rules_of[v].iter().flat_map(|r| r.deps.iter()).filter(|d| !placed.contains(d)).count();
            let next = (0..n).filter(|v| !placed.contains(v)).min_by_key(|&v| unmet(v, &placed)).unwrap();
            placed.push(next);
        }
        placed
    }

    /// The truth (1 bit) of a rule's conditions and `soft_ok` (soft rules count only in the
    /// first phase): a jump over the rule's code when it is off. Returns the patch list.
    fn rule_guard(&mut self, r: &Rule, soft_on: u32) -> R<Vec<usize>> {
        let mut skips = Vec::new();
        for &(g, pol, gs) in &r.guards {
            let e = if pol { g.clone() } else { ExprKind::Unary(UnOp::LNot, Box::new(g.clone())).into() };
            let (c, cw) = self.truth(&e, gs)?;
            skips.push(self.emit(Instr::Jz { c, w: cw, to: 0 }));
        }
        if r.soft {
            skips.push(self.emit(Instr::Jz { c: soft_on, w: 1, to: 0 }));
        }
        Ok(skips)
    }

    /// A jump (to patch) taken when the 1-bit value in slot `c` is 1.
    fn jump_if(&mut self, c: u32) -> usize {
        let n = self.tmp(1);
        self.emit(Instr::Un { op: rt::UnOp::Not, d: n, s: c, w: 1 });
        self.emit(Instr::Jz { c: n, w: 1, to: 0 })
    }

    fn patch_all(&mut self, v: Vec<usize>) {
        let h = self.here();
        for j in v {
            self.patch(j, h);
        }
    }

    /// Store the `v.w`-bit value in slot `s` into variable `v`.
    fn rvar_store(&mut self, v: &RVar, s: u32, h: Option<u32>) -> R<()> {
        match &v.home {
            Home::Prop { off } => {
                self.emit(Instr::Obj { op: ObjOp::Set, d: NONE, a: h.unwrap(), b: s, off: *off, w: v.w });
            }
            Home::Var { e, sid } => self.store_value(&crate::stmt::Lhs::Expr(e, *sid), s, v.w, rt::StoreMode::Blocking)?,
        }
        Ok(())
    }

    fn rvar_load(&mut self, v: &RVar, h: Option<u32>) -> R<u32> {
        Ok(match &v.home {
            Home::Prop { off } => {
                let d = self.tmp(v.w);
                self.emit(Instr::Obj { op: ObjOp::Get, d, a: h.unwrap(), b: NONE, off: *off, w: v.w });
                d
            }
            Home::Var { e, sid } => {
                let s = self.expr(e, *sid, v.w, v.signed)?;
                let d = self.tmp(v.w);
                self.emit(Instr::Copy { d, s, w: v.w });
                d
            }
        })
    }

    /// A 64-bit value of `e` as a bound of variable `v` (sign-extended for a signed one).
    fn bound64(&mut self, e: &Expr, sid: usize, v: &RVar) -> R<u32> {
        self.expr(e, sid, 64, v.signed)
    }

    /// The solving loop: values for `rv`, `res` = 1 on success, else the old values stay
    /// and `res` = 0.
    fn solve(&mut self, rv: &[RVar], items: &[Item], h: Option<u32>, res: u32, at: &str) -> R<()> {
        let items = self.unroll(items)?;
        for i in &items {
            if let CItem::DisableSoft(_) = i.it {
                return self.err("disable soft is not supported");
            }
        }
        // the old values (restored when there is no solution)
        let old: Vec<u32> = rv.iter().map(|v| self.rvar_load(v, h)).collect::<R<_>>()?;
        let old: Vec<u32> = old
            .into_iter()
            .zip(rv)
            .map(|(s, v)| {
                let p = self.ptmp(v.w);
                self.emit(Instr::Copy { d: p, s, w: v.w });
                p
            })
            .collect();
        let tries = self.ptmp(32);
        let zero = self.const_slot(&Const::zero(32));
        self.emit(Instr::Copy { d: tries, s: zero, w: 32 });
        let soft_on = self.ptmp(1);
        let top = self.here();
        // soft constraints count for the first TRIES attempts
        let limit = self.const_slot(&Const::from_i64(TRIES, 32, true));
        self.emit(Instr::Cmp { op: rt::CmpOp::LtS, d: soft_on, a: tries, b: limit, w: 32 });
        let mut rules_of = Vec::new();
        for v in 0..rv.len() {
            let mut r = Vec::new();
            Self::rules(&items, rv, v, &mut r);
            rules_of.push(r);
        }
        let order = Self::rand_order(&rules_of);
        // the values from the rules whose variables have theirs
        for (pos, &vi) in order.iter().enumerate() {
            let v = &rv[vi];
            let before = &order[..pos];
            let rules: Vec<&Rule> = rules_of[vi].iter().filter(|r| r.deps.iter().all(|d| before.contains(d))).collect();
            let val = self.tmp(v.w.max(64));
            let picked = self.tmp(1);
            let z1 = self.const_slot(&Const::zero(1));
            self.emit(Instr::Copy { d: picked, s: z1, w: 1 });
            // a set (inside, dist) whose conditions hold
            for r in rules.iter().copied().filter(|r| matches!(r.kind, RuleKind::Inside(_) | RuleKind::Dist(_))) {
                let mut skips = vec![self.jump_if(picked)];
                skips.extend(self.rule_guard(r, soft_on)?);
                match r.kind {
                    RuleKind::Inside(set) => self.pick_inside(set, r.sid, v, val)?,
                    RuleKind::Dist(d) => self.pick_dist(d, r.sid, v, val)?,
                    _ => unreachable!(),
                }
                let one = self.const_slot(&Const::from_i64(1, 1, false));
                self.emit(Instr::Copy { d: picked, s: one, w: 1 });
                self.patch_all(skips);
            }
            // otherwise a value in the range the bounds leave
            let skip = self.jump_if(picked);
            if v.w <= 64 {
                let (lo, hi) = (self.tmp(64), self.tmp(64));
                let (min, max) = if v.signed {
                    (-(1i128 << (v.w - 1)), (1i128 << (v.w - 1)) - 1)
                } else {
                    (0, (1i128 << v.w) - 1)
                };
                let c64 = |me: &mut Self, x: i128| me.const_slot(&Const::from_i64(x as i64, 64, v.signed));
                let (cmin, cmax) = (c64(self, min), c64(self, max));
                self.emit(Instr::Copy { d: lo, s: cmin, w: 64 });
                self.emit(Instr::Copy { d: hi, s: cmax, w: 64 });
                let one64 = c64(self, 1);
                for r in rules.iter().copied() {
                    let (e, is_lo, is_hi, strict) = match r.kind {
                        RuleKind::Lo(e, s) => (e, true, false, s),
                        RuleKind::Hi(e, s) => (e, false, true, s),
                        RuleKind::Eq(e) => (e, true, true, false),
                        _ => continue,
                    };
                    let skips = self.rule_guard(r, soft_on)?;
                    let b = self.bound64(e, r.sid, v)?;
                    let b2 = self.tmp(64);
                    let op = if is_lo { rt::BinOp::Add } else { rt::BinOp::Sub };
                    if strict {
                        self.emit(Instr::Bin { op, d: b2, a: b, b: one64, w: 64 });
                    } else {
                        self.emit(Instr::Copy { d: b2, s: b, w: 64 });
                    }
                    let lt = if v.signed { rt::CmpOp::LtS } else { rt::CmpOp::Lt };
                    if is_lo {
                        // lo = max(lo, b)
                        let c = self.tmp(1);
                        self.emit(Instr::Cmp { op: lt, d: c, a: lo, b: b2, w: 64 });
                        let j = self.emit(Instr::Jz { c, w: 1, to: 0 });
                        self.emit(Instr::Copy { d: lo, s: b2, w: 64 });
                        self.patch_all(vec![j]);
                    }
                    if is_hi {
                        let c = self.tmp(1);
                        self.emit(Instr::Cmp { op: lt, d: c, a: b2, b: hi, w: 64 });
                        let j = self.emit(Instr::Jz { c, w: 1, to: 0 });
                        self.emit(Instr::Copy { d: hi, s: b2, w: 64 });
                        self.patch_all(vec![j]);
                    }
                    self.patch_all(skips);
                }
                let obj = self.code.rand_obj.unwrap_or(NONE);
                let f = match v.cyc {
                    Some(key) => SysFn::RandcPick { lo, hi, w: v.w, signed: v.signed, obj, key },
                    None => SysFn::RandRange { lo, hi, w: v.w, signed: v.signed, obj },
                };
                self.emit(Instr::SysF { d: val, f });
            } else {
                self.emit(Instr::SysF { d: val, f: SysFn::RandRange { lo: NONE, hi: NONE, w: v.w, signed: false, obj: self.code.rand_obj.unwrap_or(NONE) } });
            }
            self.patch_all(vec![skip]);
            self.rvar_store(v, val, h)?;
        }
        // all constraints hold (the soft ones in the first phase)?
        let ok = self.tmp(1);
        let one = self.const_slot(&Const::from_i64(1, 1, false));
        self.emit(Instr::Copy { d: ok, s: one, w: 1 });
        for i in &items {
            let (c, soft) = self.item_truth(&i.it, i.sid)?;
            let c1 = self.tmp(1);
            self.emit(Instr::Red { op: rt::RedOp::Bool, d: c1, s: c.0, w: c.1 });
            // (x: false)
            let known = self.tmp(1);
            self.emit(Instr::Un { op: rt::UnOp::To2, d: known, s: c1, w: 1 });
            if soft {
                // soft: (!soft_on || c)
                let ns = self.tmp(1);
                self.emit(Instr::Un { op: rt::UnOp::Not, d: ns, s: soft_on, w: 1 });
                self.emit(Instr::Bin { op: rt::BinOp::Or, d: known, a: known, b: ns, w: 1 });
            }
            self.emit(Instr::Bin { op: rt::BinOp::And, d: ok, a: ok, b: known, w: 1 });
        }
        // the randc variables' values not given yet in their cycles, in the first half of
        // each phase (then a value of a cycle that constraints leave no value of starts a
        // new one)
        if rv.iter().any(|v| v.cyc.is_some()) {
            let ph = self.tmp(32);
            let t = self.const_slot(&Const::from_i64(TRIES, 32, true));
            self.emit(Instr::Bin { op: rt::BinOp::Mod, d: ph, a: tries, b: t, w: 32 });
            let half = self.const_slot(&Const::from_i64(TRIES / 2, 32, true));
            let first = self.tmp(1);
            self.emit(Instr::Cmp { op: rt::CmpOp::Lt, d: first, a: ph, b: half, w: 32 });
            let skip = self.emit(Instr::Jz { c: first, w: 1, to: 0 });
            for v in rv {
                let Some(key) = v.cyc else { continue };
                let x = self.rvar_load(v, h)?;
                let free = self.tmp(1);
                let obj = self.code.rand_obj.unwrap_or(NONE);
                self.emit(Instr::SysF { d: free, f: SysFn::RandcFree { v: x, w: v.w, obj, key } });
                self.emit(Instr::Bin { op: rt::BinOp::And, d: ok, a: ok, b: free, w: 1 });
            }
            self.patch_all(vec![skip]);
        }
        let done = self.jump_if(ok);
        // another attempt
        let one32 = self.const_slot(&Const::from_i64(1, 32, true));
        self.emit(Instr::Bin { op: rt::BinOp::Add, d: tries, a: tries, b: one32, w: 32 });
        let all = self.const_slot(&Const::from_i64(2 * TRIES, 32, true));
        let more = self.tmp(1);
        self.emit(Instr::Cmp { op: rt::CmpOp::LtS, d: more, a: tries, b: all, w: 32 });
        let again = self.emit(Instr::Jz { c: more, w: 1, to: 0 });
        self.emit(Instr::Jmp { to: top });
        self.patch_all(vec![again]);
        // no solution: the old values, 0 and a warning
        for (v, &o) in rv.iter().zip(&old) {
            self.rvar_store(v, o, h)?;
        }
        let z = self.const_slot(&Const::zero(32));
        self.emit(Instr::Copy { d: res, s: z, w: 32 });
        let msg = format!("{at}: warning: randomize() found no values that satisfy the constraints");
        let scope = String::new();
        self.emit(Instr::Sys(Box::new(rt::SysTask::Display { nl: true, args: vec![rt::FmtArg::Lit(msg)], scope, fd: None, prefix: "", texp: 0, radix: 'd' })));
        let fail_end = self.emit(Instr::Jmp { to: 0 });
        self.patch_all(vec![done]);
        for v in rv {
            let Some(key) = v.cyc else { continue };
            let x = self.rvar_load(v, h)?;
            let d = self.tmp(1);
            let obj = self.code.rand_obj.unwrap_or(NONE);
            self.emit(Instr::SysF { d, f: SysFn::RandcMark { v: x, w: v.w, obj, key } });
        }
        let one32 = self.const_slot(&Const::from_i64(1, 32, true));
        self.emit(Instr::Copy { d: res, s: one32, w: 32 });
        self.patch_all(vec![fail_end]);
        Ok(())
    }

    /// A value of `set` (`inside`) into `val`: one of its items at random, a value of a range
    /// at random.
    fn pick_inside(&mut self, set: &[(Expr, Option<Expr>)], sid: usize, v: &RVar, val: u32) -> R<()> {
        let n = set.len() as i64;
        let (lo, hi) = (self.const_slot(&Const::zero(64)), self.const_slot(&Const::from_i64(n - 1, 64, false)));
        let k = self.tmp(64);
        self.emit(Instr::SysF { d: k, f: SysFn::RandRange { lo, hi, w: 64, signed: false, obj: self.code.rand_obj.unwrap_or(NONE) } });
        for (j, (a, b)) in set.iter().enumerate() {
            let kj = self.const_slot(&Const::from_i64(j as i64, 64, false));
            let is = self.tmp(1);
            self.emit(Instr::Cmp { op: rt::CmpOp::Eq, d: is, a: k, b: kj, w: 64 });
            let skip = self.emit(Instr::Jz { c: is, w: 1, to: 0 });
            self.range_value(a, b.as_ref(), sid, v, val)?;
            self.patch_all(vec![skip]);
        }
        Ok(())
    }

    /// A value of `[a:b]` (or `a`) into `val`.
    fn range_value(&mut self, a: &Expr, b: Option<&Expr>, sid: usize, v: &RVar, val: u32) -> R<()> {
        let w = v.w.max(64);
        match b {
            None => {
                let s = self.expr(a, sid, w, v.signed)?;
                self.emit(Instr::Copy { d: val, s, w });
            }
            Some(b) => {
                let (x, y) = (self.bound64(a, sid, v)?, self.bound64(b, sid, v)?);
                // (either order)
                let (lo, hi) = (self.tmp(64), self.tmp(64));
                let lt = if v.signed { rt::CmpOp::LtS } else { rt::CmpOp::Lt };
                let c = self.tmp(1);
                self.emit(Instr::Cmp { op: lt, d: c, a: y, b: x, w: 64 });
                self.emit(Instr::Copy { d: lo, s: x, w: 64 });
                self.emit(Instr::Copy { d: hi, s: y, w: 64 });
                let j = self.emit(Instr::Jz { c, w: 1, to: 0 });
                self.emit(Instr::Copy { d: lo, s: y, w: 64 });
                self.emit(Instr::Copy { d: hi, s: x, w: 64 });
                self.patch_all(vec![j]);
                let obj = self.code.rand_obj.unwrap_or(NONE);
                let f = match v.cyc {
                    Some(key) => SysFn::RandcPick { lo, hi, w: v.w, signed: v.signed, obj, key },
                    None => SysFn::RandRange { lo, hi, w: v.w.min(64), signed: v.signed, obj },
                };
                self.emit(Instr::SysF { d: val, f });
            }
        }
        Ok(())
    }

    /// A value of `dist` items into `val`, by their weights.
    fn pick_dist(&mut self, items: &[ferrooxide_front::ast::DistItem], sid: usize, v: &RVar, val: u32) -> R<()> {
        // each item's weight (`:=` of a range: per value), and their running sum
        let total = self.tmp(64);
        let z = self.const_slot(&Const::zero(64));
        self.emit(Instr::Copy { d: total, s: z, w: 64 });
        let mut ends = Vec::new();
        for d in items {
            let w = self.expr(&d.weight, sid, 64, false)?;
            let wt = self.tmp(64);
            match (&d.hi, d.per_value) {
                (Some(hi), true) => {
                    let size = bin(BinOp::Add, bin(BinOp::Sub, hi.clone(), d.lo.clone()), num(1));
                    let s = self.expr(&size, sid, 64, false)?;
                    self.emit(Instr::Bin { op: rt::BinOp::Mul, d: wt, a: w, b: s, w: 64 });
                }
                _ => {
                    self.emit(Instr::Copy { d: wt, s: w, w: 64 });
                }
            }
            let e = self.tmp(64);
            self.emit(Instr::Bin { op: rt::BinOp::Add, d: e, a: total, b: wt, w: 64 });
            self.emit(Instr::Copy { d: total, s: e, w: 64 });
            ends.push(e);
        }
        // r in [0, total): the first item whose running sum exceeds it
        let r = self.tmp(64);
        let hi = self.tmp(64);
        let one = self.const_slot(&Const::from_i64(1, 64, false));
        self.emit(Instr::Bin { op: rt::BinOp::Sub, d: hi, a: total, b: one, w: 64 });
        self.emit(Instr::SysF { d: r, f: SysFn::RandRange { lo: z, hi, w: 64, signed: false, obj: self.code.rand_obj.unwrap_or(NONE) } });
        let mut out = Vec::new();
        for (d, e) in items.iter().zip(ends) {
            let c = self.tmp(1);
            self.emit(Instr::Cmp { op: rt::CmpOp::Lt, d: c, a: r, b: e, w: 64 });
            let skip = self.emit(Instr::Jz { c, w: 1, to: 0 });
            self.range_value(&d.lo, d.hi.as_ref(), sid, v, val)?;
            out.push(self.emit(Instr::Jmp { to: 0 }));
            self.patch_all(vec![skip]);
        }
        self.patch_all(out);
        Ok(())
    }

    /// The truth of a constraint item: (slot, width) and whether it is soft.
    fn item_truth(&mut self, it: &CItem, sid: usize) -> R<((u32, u32), bool)> {
        let e = Self::item_expr(it);
        let soft = matches!(it, CItem::Expr { soft: true, .. } | CItem::Inside { soft: true, .. } | CItem::Dist { soft: true, .. });
        Ok((self.truth(&e, sid)?, soft))
    }

    /// A constraint item as a boolean expression.
    fn item_expr(it: &CItem) -> Expr {
        let all = |v: &[CItem]| v.iter().map(Self::item_expr).reduce(|a, b| bin(BinOp::LAnd, a, b)).unwrap_or(num(1));
        let in_range = |e: &Expr, a: &Expr, b: &Option<Expr>| match b {
            None => bin(BinOp::Eq, e.clone(), a.clone()),
            // (either order)
            Some(b) => bin(
                BinOp::LOr,
                bin(BinOp::LAnd, bin(BinOp::Ge, e.clone(), a.clone()), bin(BinOp::Le, e.clone(), b.clone())),
                bin(BinOp::LAnd, bin(BinOp::Ge, e.clone(), b.clone()), bin(BinOp::Le, e.clone(), a.clone())),
            ),
        };
        match it {
            CItem::Expr { e, .. } => e.clone(),
            CItem::Inside { e, set, .. } => set.iter().map(|(a, b)| in_range(e, a, b)).reduce(|a, b| bin(BinOp::LOr, a, b)).unwrap_or(num(0)),
            // (an item of weight 0 is no value of the variable)
            CItem::Dist { e, items, .. } => items
                .iter()
                .map(|d| bin(BinOp::LAnd, bin(BinOp::Ne, d.weight.clone(), num(0)), in_range(e, &d.lo, &d.hi)))
                .reduce(|a, b| bin(BinOp::LOr, a, b))
                .unwrap_or(num(0)),
            CItem::Implies(g, v) => bin(BinOp::LOr, ExprKind::Unary(UnOp::LNot, Box::new(g.clone())).into(), all(v)),
            CItem::If(g, a, b) => ExprKind::Cond(Box::new(g.clone()), Box::new(all(a)), Box::new(all(b))).into(),
            CItem::Unique(v) => {
                let mut acc = num(1);
                for i in 0..v.len() {
                    for j in i + 1..v.len() {
                        acc = bin(BinOp::LAnd, acc, bin(BinOp::Ne, v[i].clone(), v[j].clone()));
                    }
                }
                acc
            }
            // (unrolled before)
            CItem::Foreach { .. } | CItem::SolveBefore | CItem::DisableSoft(_) => num(1),
        }
    }
}
