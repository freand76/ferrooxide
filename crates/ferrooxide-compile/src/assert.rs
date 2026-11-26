//! Concurrent assertions (IEEE 1800-2017 16): a property becomes automata over the ticks of
//! its clock (`rt::assert`) and a process that, at each tick, evaluates the boolean leaves on
//! sampled values, steps the assertion and runs the action blocks in the Reactive region.
//!
//! Sampled values (16.5.1) are copies of the signals the leaves read, made before each time
//! step (`Program::samples`); a leaf reads the copy (`$smp<s>`). `$past` is a chain of
//! registers shifted at each tick; `$rose`, `$fell`, `$stable` and `$changed` compare the
//! sampled value with `$past`. `disable iff` reads current values: a second process ends the
//! running attempts when it becomes true, and no attempt starts at a tick where it is.

use crate::*;
use ferrooxide_rt::assert::{AssertDef, Lit, Nfa, PropDef};
use std::collections::{HashMap, VecDeque};

/// The most states an assertion's automata may have.
const MAX_STATES: usize = 1 << 16;

/// A sequence being built: its automaton and whether it matches the empty sequence.
#[derive(Clone)]
struct Frag {
    nfa: Nfa,
    empty: bool,
}

impl Frag {
    fn n(&self) -> u32 {
        self.nfa.accept.len() as u32
    }

    /// No tick, the empty match.
    fn empty() -> Frag {
        Frag { nfa: Nfa::default(), empty: true }
    }

    /// One tick whose leaves satisfy `cond`.
    fn tick(cond: Vec<Lit>) -> Frag {
        Frag { nfa: Nfa { start: vec![0], accept: vec![false, true], trans: vec![vec![(cond, 1)], vec![]] }, empty: false }
    }

    /// `b`'s states after `a`'s: (a's automaton with room for b, the offset of b's states).
    fn joined(a: &Frag, b: &Frag) -> (Nfa, u32) {
        let off = a.n();
        let mut n = a.nfa.clone();
        n.accept.extend(b.nfa.accept.iter().copied());
        n.trans.extend(b.nfa.trans.iter().map(|ts| ts.iter().map(|(c, t)| (c.clone(), t + off)).collect::<Vec<_>>()));
        (n, off)
    }

    /// `a or b`
    fn or(a: &Frag, b: &Frag) -> Frag {
        let (mut n, off) = Frag::joined(a, b);
        n.start.extend(b.nfa.start.iter().map(|s| s + off));
        Frag { nfa: n, empty: a.empty || b.empty }
    }

    /// `a ##1 b`
    fn concat(a: &Frag, b: &Frag) -> Frag {
        let (mut n, off) = Frag::joined(a, b);
        let firsts: Vec<(Vec<Lit>, u32)> = b.nfa.start.iter().flat_map(|&s| b.nfa.trans[s as usize].iter().map(|(c, t)| (c.clone(), t + off))).collect();
        for s in 0..a.n() as usize {
            if a.nfa.accept[s] {
                n.trans[s].extend(firsts.iter().cloned());
                n.accept[s] = b.empty;
            }
        }
        // (`empty ##1 b` is b)
        if a.empty {
            n.start.extend(b.nfa.start.iter().map(|s| s + off));
        }
        Frag { nfa: n, empty: a.empty && b.empty }
    }

    /// `a ##0 b`: b's first tick is a's last (no match with an empty side)
    fn fuse(a: &Frag, b: &Frag) -> Frag {
        let (mut n, off) = Frag::joined(a, b);
        let firsts: Vec<(Vec<Lit>, u32)> = b.nfa.start.iter().flat_map(|&s| b.nfa.trans[s as usize].iter().map(|(c, t)| (c.clone(), t + off))).collect();
        for s in 0..a.n() as usize {
            let mut add = Vec::new();
            for (c, t) in &a.nfa.trans[s] {
                if a.nfa.accept[*t as usize] {
                    for (c2, t2) in &firsts {
                        if let Some(c) = and_cond(c, c2) {
                            add.push((c, *t2));
                        }
                    }
                }
            }
            n.trans[s].extend(add);
        }
        for s in 0..a.n() as usize {
            n.accept[s] = false;
        }
        Frag { nfa: n, empty: false }
    }

    /// `a[*0:$]`
    fn star(a: &Frag) -> Frag {
        let mut n = a.nfa.clone();
        let firsts: Vec<(Vec<Lit>, u32)> = a.nfa.start.iter().flat_map(|&s| a.nfa.trans[s as usize].clone()).collect();
        for s in 0..a.n() as usize {
            if a.nfa.accept[s] {
                n.trans[s].extend(firsts.iter().cloned());
            }
        }
        Frag { nfa: n, empty: true }
    }

    /// `a intersect b`: both match, of the same length
    fn intersect(a: &Frag, b: &Frag) -> Frag {
        let mut ids: HashMap<(u32, u32), u32> = HashMap::new();
        let mut q = VecDeque::new();
        let mut n = Nfa::default();
        let mut id = |p: (u32, u32), n: &mut Nfa, q: &mut VecDeque<(u32, u32)>| -> u32 {
            *ids.entry(p).or_insert_with(|| {
                n.accept.push(a.nfa.accept[p.0 as usize] && b.nfa.accept[p.1 as usize]);
                n.trans.push(Vec::new());
                q.push_back(p);
                n.accept.len() as u32 - 1
            })
        };
        for &x in &a.nfa.start {
            for &y in &b.nfa.start {
                let s = id((x, y), &mut n, &mut q);
                n.start.push(s);
            }
        }
        while let Some((x, y)) = q.pop_front() {
            let from = id((x, y), &mut n, &mut q);
            for (c1, t1) in &a.nfa.trans[x as usize] {
                for (c2, t2) in &b.nfa.trans[y as usize] {
                    if let Some(c) = and_cond(c1, c2) {
                        let to = id((*t1, *t2), &mut n, &mut q);
                        n.trans[from as usize].push((c, to));
                    }
                }
            }
        }
        Frag { nfa: n, empty: a.empty && b.empty }
    }

    /// Without the states that are not reachable or cannot reach a match (so that an attempt
    /// ends as soon as it cannot match any more).
    fn prune(self) -> Frag {
        let n = self.n() as usize;
        let mut fwd = vec![false; n];
        let mut q: VecDeque<u32> = self.nfa.start.iter().copied().collect();
        for &s in &self.nfa.start {
            fwd[s as usize] = true;
        }
        while let Some(s) = q.pop_front() {
            for (_, t) in &self.nfa.trans[s as usize] {
                if !fwd[*t as usize] {
                    fwd[*t as usize] = true;
                    q.push_back(*t);
                }
            }
        }
        let mut back = self.nfa.accept.clone();
        loop {
            let mut changed = false;
            for s in 0..n {
                if !back[s] && self.nfa.trans[s].iter().any(|(_, t)| back[*t as usize]) {
                    back[s] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let keep: Vec<bool> = (0..n).map(|s| fwd[s] && back[s]).collect();
        let mut map = vec![u32::MAX; n];
        let mut k = 0;
        for s in 0..n {
            if keep[s] {
                map[s] = k;
                k += 1;
            }
        }
        let mut out = Nfa::default();
        for s in 0..n {
            if keep[s] {
                out.accept.push(self.nfa.accept[s]);
                out.trans.push(self.nfa.trans[s].iter().filter(|(_, t)| keep[*t as usize]).map(|(c, t)| (c.clone(), map[*t as usize])).collect());
            }
        }
        out.start = self.nfa.start.iter().filter(|s| keep[**s as usize]).map(|s| map[*s as usize]).collect();
        Frag { nfa: out, empty: self.empty }
    }
}

/// The conjunction of two conditions (None: contradictory).
fn and_cond(a: &[Lit], b: &[Lit]) -> Option<Vec<Lit>> {
    let mut c = a.to_vec();
    for &(l, pos) in b {
        match c.iter().find(|x| x.0 == l) {
            Some(x) if x.1 != pos => return None,
            Some(_) => {}
            None => c.push((l, pos)),
        }
    }
    c.sort_unstable();
    Some(c)
}

/// An assertion being compiled: its leaves (on sampled values) and automata.
#[derive(Default)]
struct Build {
    leaves: Vec<Expr>,
    keys: HashMap<String, u32>,
    /// `$past` registers shifted at each tick: (value, registers newest first)
    pasts: Vec<(Expr, Vec<String>)>,
    def: AssertDef,
    /// the assertion reads `$future` values: each tick evaluates the attempt of the tick
    /// before, signals reading their values of one tick ago (1) and `$future` the current
    /// ones (0)
    shift: u32,
}

impl<'a> Compiler<'a> {
    /// A concurrent assertion in scope `sid`: its definition and processes.
    pub(crate) fn assertion(&mut self, a: &ConcAssert, sid: usize) -> R<()> {
        self.line = a.line;
        let mut b = Build { shift: u32::from(format!("{:?}", a.prop).contains("\"$future\"")), ..Build::default() };
        let future = b.shift > 0;
        let top = self.prop_def(&a.prop, &mut b, sid)?;
        debug_assert_eq!(top as usize, b.def.props.len() - 1);
        b.def.cover = a.kind == AssertKind::Cover;
        b.def.all_matches = a.kind == AssertKind::CoverSeq;
        let id = self.prog.asserts.len() as u32;
        self.prog.asserts.push(std::mem::take(&mut b.def));
        // the numbers of passes and failures of a tick (for the action blocks)
        let counter = |c: &mut Self, what: &str| -> R<(String, u32)> {
            let name = format!("$assert{id}{what}");
            let full = format!("{}.{name}", c.scopes[sid].name);
            let s = c.new_signal(full, 32, false, 0, 31, 0, 0, SigKind::Reg);
            c.prog.names.pop();
            c.scopes[sid].syms.insert(name.clone(), Sym::Sig(s));
            Ok((name, c.sig(s).slot))
        };
        let (pass_n, pass_slot) = counter(self, "pass")?;
        let (fail_n, fail_slot) = counter(self, "fail")?;
        let name = self.scopes[sid].name.clone();
        self.begin_proc(name.clone(), ProcClass::Normal);
        let top_pc = self.here();
        self.stmt(&Stmt::Timed { timing: Timing::Event(a.clock.clone()), body: Box::new(Stmt::Null), line: a.line }, sid)?;
        // (with `$future` values: no attempt at the first tick, it ends at the second)
        let mut first = None;
        if future {
            let (started, _) = counter(self, "started")?;
            let (c, cw) = self.truth(&ExprKind::Ident(started.clone()).into(), sid)?;
            first = Some((self.emit(Instr::Jz { c, w: cw, to: 0 }), started));
        }
        // `disable iff` true at the tick: no attempt starts, the running ones end (no
        // passes or failures; the sampled value functions go on)
        let mut skip = None;
        if let Some(d) = &a.disable {
            let (c, cw) = self.truth(d, sid)?;
            let j = self.emit(Instr::Jz { c, w: cw, to: 0 });
            self.emit(Instr::Assert(rt::AssertOp::Disable { id }));
            let z = self.const_slot(&Const::zero(32));
            self.emit(Instr::Copy { d: pass_slot, s: z, w: 32 });
            self.emit(Instr::Copy { d: fail_slot, s: z, w: 32 });
            skip = Some(self.emit(Instr::Jmp { to: 0 }));
            let here = self.here();
            self.patch(j, here);
        }
        let n = b.leaves.len() as u32;
        let leaves = if n == 0 {
            NONE
        } else {
            // leaf i is bit i
            let v: Expr = ExprKind::Concat(b.leaves.iter().rev().cloned().collect()).into();
            self.expr(&v, sid, n, false)?
        };
        self.emit(Instr::Assert(rt::AssertOp::Tick { id, leaves, n, pass: pass_slot, fail: fail_slot }));
        if let Some((j, started)) = first {
            let over = self.emit(Instr::Jmp { to: 0 });
            let here = self.here();
            self.patch(j, here);
            let one = ExprKind::Num(Number { size: None, signed: false, base: 10, based: false, digits: "1".into(), fill: false });
            self.stmt(&Stmt::Assign { lhs: ExprKind::Ident(started).into(), rhs: one.into(), nonblocking: false, timing: None, line: a.line }, sid)?;
            let here = self.here();
            self.patch(over, here);
        }
        if let Some(j) = skip {
            let here = self.here();
            self.patch(j, here);
        }
        // `$past` registers: oldest first, those of later entries (which may read the
        // registers of earlier ones) first
        for (v, regs) in b.pasts.iter().rev() {
            for k in (0..regs.len()).rev() {
                let rhs = if k == 0 { v.clone() } else { ExprKind::Ident(regs[k - 1].clone()).into() };
                let st = Stmt::Assign { lhs: ExprKind::Ident(regs[k].clone()).into(), rhs, nonblocking: false, timing: None, line: a.line };
                self.stmt(&st, sid)?;
            }
        }
        // the action blocks, in the Reactive region: one run per failure and per success
        let mut actions = Vec::new();
        let count = |n: &str| -> Expr { ExprKind::Ident(n.to_string()).into() };
        if !matches!(a.fail, Stmt::Null) {
            actions.push(Stmt::Repeat { count: count(&fail_n), body: Box::new(a.fail.clone()), line: a.line });
        }
        if !matches!(a.pass, Stmt::Null) {
            actions.push(Stmt::Repeat { count: count(&pass_n), body: Box::new(a.pass.clone()), line: a.line });
        }
        if !actions.is_empty() {
            let any: Expr = ExprKind::Binary(BinOp::Or, Box::new(count(&pass_n)), Box::new(count(&fail_n))).into();
            let (c, cw) = self.truth(&any, sid)?;
            self.emit(Instr::Jz { c, w: cw, to: top_pc });
            self.emit(Instr::Assert(rt::AssertOp::Reactive));
            let body = Stmt::Block { name: a.label.clone(), decls: vec![], imports: vec![], params: vec![], stmts: actions, fork: None };
            self.stmt(&body, sid)?;
        }
        self.emit(Instr::Jmp { to: top_pc });
        self.end_proc();
        // `disable iff` between ticks
        if let Some(d) = &a.disable {
            self.begin_proc(name, ProcClass::Normal);
            let ctl = self.make_ctl(Vec::new());
            let top = self.here();
            self.emit(Instr::Wait { ctl });
            let (c, cw) = self.truth(d, sid)?;
            self.emit(Instr::Jz { c, w: cw, to: top });
            self.emit(Instr::Assert(rt::AssertOp::Disable { id }));
            self.emit(Instr::Jmp { to: top });
            let mut reads = Vec::new();
            self.reads_expr(d, sid, &mut reads);
            let mut trig: Vec<(u32, rt::Edge, u32)> = reads.into_iter().map(|(s, e)| (s, rt::Edge::Any, e)).collect();
            trig.sort_by_key(|t| (t.0, t.2, t.1 as u8));
            trig.dedup();
            self.prog.ctls[ctl as usize].triggers = trig;
            self.end_proc();
        }
        Ok(())
    }

    /// Property `p` as `PropDef`s; returns the index of its top.
    fn prop_def(&mut self, p: &Property, b: &mut Build, sid: usize) -> R<u32> {
        let d = match p {
            Property::Seq(s) => {
                let f = self.seq_frag(s, b, sid)?.prune();
                if f.empty {
                    return self.err("a sequence used as a property must not match the empty sequence");
                }
                PropDef::Seq(self.add_nfa(f, b)?)
            }
            Property::Not(a) => PropDef::Not(self.prop_def(a, b, sid)?),
            Property::And(x, y) => PropDef::And(self.prop_def(x, b, sid)?, self.prop_def(y, b, sid)?),
            Property::Or(x, y) => PropDef::Or(self.prop_def(x, b, sid)?, self.prop_def(y, b, sid)?),
            Property::Implies { ante, next, cons } => {
                let f = self.seq_frag(ante, b, sid)?.prune();
                if f.empty {
                    return self.unsupported("an implication whose antecedent can match the empty sequence");
                }
                let ante = self.add_nfa(f, b)?;
                PropDef::Implies { ante, next: *next, cons: self.prop_def(cons, b, sid)? }
            }
            Property::If(c, x, y) => {
                let c = self.leaf(c, b, sid)?;
                let x = self.prop_def(x, b, sid)?;
                let y = match y {
                    Some(y) => Some(self.prop_def(y, b, sid)?),
                    None => None,
                };
                PropDef::If(c, x, y)
            }
        };
        b.def.props.push(d);
        Ok(b.def.props.len() as u32 - 1)
    }

    fn unsupported<T>(&self, what: &str) -> R<T> {
        self.err(format!("{what} is not supported"))
    }

    fn add_nfa(&mut self, f: Frag, b: &mut Build) -> R<u32> {
        let total: usize = b.def.nfas.iter().map(|n| n.states()).sum::<usize>() + f.nfa.states();
        if total > MAX_STATES {
            return self.err("this property is too large (its automata have too many states)");
        }
        b.def.nfas.push(f.nfa);
        Ok(b.def.nfas.len() as u32 - 1)
    }

    /// The tick counts of `c`: (lo, hi; None: unbounded).
    fn cycles(&mut self, c: &Cycles, sid: usize) -> R<(u32, Option<u32>)> {
        let lo = self.const_int(&c.lo, sid)?;
        let hi = match (&c.hi, c.unbounded) {
            (_, true) => None,
            (Some(h), false) if self.is_unbounded(h, sid) => None,
            (Some(h), false) => Some(self.const_int(h, sid)?),
            (None, false) => Some(lo),
        };
        if lo < 0 || hi.is_some_and(|h| h < lo) {
            return self.err("a cycle range must be nonnegative and not decreasing");
        }
        if hi.unwrap_or(lo) > 4096 {
            return self.unsupported("a cycle count above 4096");
        }
        Ok((lo as u32, hi.map(|h| h as u32)))
    }

    /// `f[*lo:hi]` (hi None: unbounded)
    fn repeat(&mut self, f: &Frag, lo: u32, hi: Option<u32>) -> R<Frag> {
        let mut r = Frag::empty();
        for _ in 0..lo {
            r = Frag::concat(&r, f);
            self.check_size(&r)?;
        }
        let tail = match hi {
            None => Frag::star(f),
            Some(h) => {
                // empty, f, f f, ... (h - lo times)
                let mut t = Frag::empty();
                for _ in lo..h {
                    t = Frag::or(&Frag::empty(), &Frag::concat(f, &t)).prune();
                    self.check_size(&t)?;
                }
                t
            }
        };
        Ok(Frag::concat(&r, &tail).prune())
    }

    fn check_size(&self, f: &Frag) -> R<()> {
        if f.nfa.states() > MAX_STATES {
            return self.err("this property is too large (its automata have too many states)");
        }
        Ok(())
    }

    /// Sequence `s` as an automaton.
    fn seq_frag(&mut self, s: &Sequence, b: &mut Build, sid: usize) -> R<Frag> {
        Ok(match s {
            Sequence::Bool(e) => Frag::tick(vec![(self.leaf(e, b, sid)?, true)]),
            Sequence::Delay(x, c, y) => {
                // (a leading `##n s` counts from a tick of `1`)
                let x = match x {
                    Some(x) => self.seq_frag(x, b, sid)?,
                    None => Frag::tick(vec![]),
                };
                let y = self.seq_frag(y, b, sid)?;
                let (lo, hi) = self.cycles(c, sid)?;
                let t = Frag::tick(vec![]);
                // x ##k y (k >= 1) is x ##1 (1[*k-1] ##1 y)
                let late = if hi.is_none_or(|h| h >= 1) {
                    let gap = self.repeat(&t, lo.max(1) - 1, hi.map(|h| h - 1))?;
                    Some(Frag::concat(&x, &Frag::concat(&gap, &y)))
                } else {
                    None
                };
                let f = match (lo, late) {
                    (0, Some(l)) => Frag::or(&Frag::fuse(&x, &y), &l),
                    (0, None) => Frag::fuse(&x, &y),
                    (_, Some(l)) => l,
                    (_, None) => unreachable!(),
                };
                f.prune()
            }
            Sequence::Repeat(x, c) => {
                let x = self.seq_frag(x, b, sid)?;
                let (lo, hi) = self.cycles(c, sid)?;
                self.repeat(&x, lo, hi)?
            }
            Sequence::Goto(e, c) | Sequence::NonCons(e, c) => {
                // b[->n]: (!b[*0:$] ##1 b)[*n]; b[=n]: b[->n] ##1 !b[*0:$]
                let l = self.leaf(e, b, sid)?;
                let one = Frag::concat(&Frag::star(&Frag::tick(vec![(l, false)])), &Frag::tick(vec![(l, true)]));
                let (lo, hi) = self.cycles(c, sid)?;
                let g = self.repeat(&one, lo, hi)?;
                if matches!(s, Sequence::NonCons(..)) {
                    Frag::concat(&g, &Frag::star(&Frag::tick(vec![(l, false)]))).prune()
                } else {
                    g
                }
            }
            Sequence::Or(x, y) => Frag::or(&self.seq_frag(x, b, sid)?, &self.seq_frag(y, b, sid)?),
            Sequence::Intersect(x, y) => Frag::intersect(&self.seq_frag(x, b, sid)?, &self.seq_frag(y, b, sid)?).prune(),
            Sequence::And(x, y) => {
                // both match; the composite ends with the later one
                let (x, y) = (self.seq_frag(x, b, sid)?, self.seq_frag(y, b, sid)?);
                let any = Frag::star(&Frag::tick(vec![]));
                let a = Frag::intersect(&Frag::concat(&x, &any), &y);
                let c = Frag::intersect(&x, &Frag::concat(&y, &any));
                Frag::or(&a, &c).prune()
            }
            Sequence::Throughout(e, y) => {
                let l = self.leaf(e, b, sid)?;
                let y = self.seq_frag(y, b, sid)?;
                Frag::intersect(&Frag::star(&Frag::tick(vec![(l, true)])), &y).prune()
            }
            Sequence::Within(x, y) => {
                let (x, y) = (self.seq_frag(x, b, sid)?, self.seq_frag(y, b, sid)?);
                let any = Frag::star(&Frag::tick(vec![]));
                Frag::intersect(&Frag::concat(&Frag::concat(&any, &x), &any), &y).prune()
            }
        })
        .and_then(|f| {
            self.check_size(&f)?;
            Ok(f)
        })
    }

    /// The leaf for boolean expression `e`: true when its sampled value is known nonzero.
    fn leaf(&mut self, e: &Expr, b: &mut Build, sid: usize) -> R<u32> {
        let s = self.at(e.line, |c| c.sampled(e, b, sid))?;
        let one = ExprKind::Num(Number { size: Some(1), signed: false, base: 2, based: true, digits: "1".into(), fill: false });
        let t: Expr = ExprKind::Binary(BinOp::CaseEq, Box::new(ExprKind::Unary(UnOp::RedOr, Box::new(s)).into()), Box::new(one.into())).into();
        let key = format!("{t:?}");
        if let Some(&k) = b.keys.get(&key) {
            return Ok(k);
        }
        let k = b.leaves.len() as u32;
        b.leaves.push(t);
        b.keys.insert(key, k);
        Ok(k)
    }

    /// `e` reading sampled values: signals as their copies, sampled value functions as
    /// registers of past values.
    fn sampled(&mut self, e: &Expr, b: &mut Build, sid: usize) -> R<Expr> {
        let line = e.line;
        let at = |k: ExprKind| Expr::new(k, line);
        match &e.kind {
            ExprKind::Ident(_) | ExprKind::Hier(_) => {
                let sym = match &e.kind {
                    ExprKind::Ident(n) => self.lookup(n, sid),
                    ExprKind::Hier(p) => self.lookup_hier(p, sid),
                    _ => unreachable!(),
                };
                let Some(Sym::Sig(s)) = sym else { return Ok(e.clone()) };
                let v = at(ExprKind::Ident(self.sample_sig(s, sid)?));
                if b.shift == 0 {
                    return Ok(v);
                }
                let n = b.shift;
                Ok(at(self.past_regs(&v, n, b, sid)?.kind))
            }
            // the sampled value at the next tick (16.9.4): the current one, the attempt being
            // that of the tick before
            ExprKind::SysCall(f, args) if f == "$future" => {
                let [x] = args.as_slice() else { return self.err("$future_gclk takes one argument") };
                if b.shift == 0 {
                    return self.err("$future_gclk inside $future_gclk");
                }
                let saved = std::mem::replace(&mut b.shift, 0);
                let r = self.sampled(x, b, sid);
                b.shift = saved;
                r
            }
            ExprKind::SysCall(f, args) if matches!(f.as_str(), "$past" | "$rose" | "$fell" | "$stable" | "$changed" | "$sampled") => {
                let Some(x) = args.first() else { return self.err(format!("{f} needs an argument")) };
                let max = if f == "$past" { 2 } else { 1 };
                if args.len() > max {
                    return self.unsupported(&format!("{f} with a gating expression or a clocking event"));
                }
                let v = self.sampled(x, b, sid)?;
                if f == "$sampled" {
                    return Ok(v);
                }
                let n = match args.get(1) {
                    Some(k) => self.const_int(k, sid)?,
                    None => 1,
                };
                if !(1..=4096).contains(&n) {
                    return self.err(format!("the number of ticks of $past must be from 1 to 4096, not {n}"));
                }
                let past = self.past_regs(&v, n as u32, b, sid)?;
                if f == "$past" {
                    return Ok(at(past.kind));
                }
                let lsb = |v: Expr| -> Expr { ExprKind::Binary(BinOp::And, Box::new(v), Box::new(ExprKind::Num(Number { size: Some(1), signed: false, base: 2, based: true, digits: "1".into(), fill: false }).into())).into() };
                let bit = |d: &str| -> Expr { ExprKind::Num(Number { size: Some(1), signed: false, base: 2, based: true, digits: d.into(), fill: false }).into() };
                let cmp = |op, a: Expr, b: Expr| -> Expr { ExprKind::Binary(op, Box::new(a), Box::new(b)).into() };
                Ok(match f.as_str() {
                    // (on the least significant bit: from not 1 to 1, from not 0 to 0)
                    "$rose" | "$fell" => {
                        let d = if f == "$rose" { "1" } else { "0" };
                        let now = cmp(BinOp::CaseEq, lsb(v), bit(d));
                        let before = cmp(BinOp::CaseNe, lsb(past), bit(d));
                        at(ExprKind::Binary(BinOp::LAnd, Box::new(now), Box::new(before)))
                    }
                    "$stable" => at(ExprKind::Binary(BinOp::CaseEq, Box::new(v), Box::new(past))),
                    _ => at(ExprKind::Binary(BinOp::CaseNe, Box::new(v), Box::new(past))),
                })
            }
            _ => {
                let mut out = e.clone();
                let mut err = None;
                out.for_each_child_mut(&mut |c| {
                    if err.is_none() {
                        match self.sampled(c, b, sid) {
                            Ok(x) => *c = x,
                            Err(x) => err = Some(x),
                        }
                    }
                });
                match err {
                    Some(x) => Err(x),
                    None => Ok(out),
                }
            }
        }
    }

    /// `$sampled(e)` outside a concurrent assertion (an action block, procedural code): `e`
    /// reading the values of the signals in the Preponed region of the time step (IEEE
    /// 1800-2017 16.9.3). None for any other expression.
    pub(crate) fn sampled_call(&mut self, e: &Expr, sid: usize) -> R<Option<Expr>> {
        let ExprKind::SysCall(f, args) = &e.kind else { return Ok(None) };
        if f != "$sampled" {
            return Ok(None);
        }
        let [x] = args.as_slice() else { return self.err("$sampled takes one argument") };
        Ok(Some(self.sampled_plain(x, sid)?))
    }

    fn sampled_plain(&mut self, e: &Expr, sid: usize) -> R<Expr> {
        let sym = match &e.kind {
            ExprKind::Ident(n) => self.lookup(n, sid),
            ExprKind::Hier(p) => self.lookup_hier(p, sid),
            ExprKind::SysCall(f, _) if matches!(f.as_str(), "$past" | "$rose" | "$fell" | "$stable" | "$changed" | "$sampled") => {
                return self.unsupported(&format!("{f} inside $sampled outside a concurrent assertion"));
            }
            ExprKind::Call(..) | ExprKind::MethodCall(..) => return self.unsupported("$sampled of a function call outside a concurrent assertion"),
            _ => None,
        };
        if let Some(Sym::Sig(s)) = sym {
            return Ok(Expr::new(ExprKind::Ident(self.sample_sig(s, sid)?), e.line));
        }
        let mut out = e.clone();
        let mut err = None;
        out.for_each_child_mut(&mut |c| {
            if err.is_none() {
                match self.sampled_plain(c, sid) {
                    Ok(x) => *c = x,
                    Err(x) => err = Some(x),
                }
            }
        });
        match err {
            Some(x) => Err(x),
            None => Ok(out),
        }
    }

    /// The sampled copy of signal `s`, by its name in scope `sid` (`$smp<s>`).
    fn sample_sig(&mut self, s: u32, sid: usize) -> R<String> {
        let name = format!("$smp{s}");
        if self.scopes[sid].syms.contains_key(&name) {
            return Ok(name);
        }
        if matches!(self.sig_type(s), types::Type::Dyn { .. } | types::Type::Class(_) | types::Type::Str) {
            return self.unsupported("a queue, dynamic or associative array, string or class handle in a concurrent assertion");
        }
        let c = match self.sigs.sampled.get(&s) {
            Some(&c) => c,
            None => {
                let i = self.sig(s).clone();
                let words = 2 * rt::nw(i.width) as u64 * i.depth.max(1) as u64;
                if words > 1 << 20 {
                    return self.unsupported(&format!("a concurrent assertion reading '{}' (more than 8 MB)", i.name));
                }
                let c = self.new_signal(format!("{}$sampled", i.name), i.width, i.signed, i.depth, i.msb, i.lsb, i.arr_lo, i.kind);
                self.prog.names.pop();
                let ty = self.sig_type(s);
                self.sigs.types[c as usize] = ty;
                if i.real {
                    self.mark_real(c);
                }
                if self.sigs.two_state.contains(&s) {
                    self.sigs.two_state.insert(c);
                }
                if let Some(d) = self.sigs.md_dims.get(&s).cloned() {
                    self.sigs.md_dims.insert(c, d);
                }
                if let Some(d) = self.sigs.packed_dims.get(&s).cloned() {
                    self.sigs.packed_dims.insert(c, d);
                }
                if let Some(l) = self.sigs.layout.get(&s).cloned() {
                    self.sigs.layout.insert(c, l);
                }
                if self.sigs.desc_arrays.contains(&s) {
                    self.sigs.desc_arrays.insert(c);
                }
                let (from, to) = (self.sig(self.canon(s)).slot, self.sig(c).slot);
                self.prog.samples.push((from, to, words as u32));
                self.sigs.sampled.insert(s, c);
                c
            }
        };
        self.scopes[sid].syms.insert(name.clone(), Sym::Sig(c));
        if self.sigs.md_dims.contains_key(&c) || self.sigs.packed_dims.contains_key(&c) {
            self.scopes[sid].syms.insert(format!("$flat:{name}"), Sym::Sig(c));
        }
        Ok(name)
    }

    /// `n` registers of the past values of `v` (sampled), shifted at each tick; returns the
    /// oldest (the value `n` ticks ago). They start as the type's default value.
    fn past_regs(&mut self, v: &Expr, n: u32, b: &mut Build, sid: usize) -> R<Expr> {
        let key = format!("{v:?}");
        let ty = self.type_of(v, sid)?;
        let (w, signed) = match ty {
            types::Type::Vec { w, signed } => (w, signed),
            types::Type::Real => return self.unsupported("a sampled value function of a real value"),
            _ => return self.unsupported("a sampled value function of this type"),
        };
        if let Some((_, regs)) = b.pasts.iter().find(|(x, r)| format!("{x:?}") == key && r.len() >= n as usize) {
            return Ok(ExprKind::Ident(regs[n as usize - 1].clone()).into());
        }
        let two_state = self.is_two_state_expr(v, sid);
        let k = b.pasts.len();
        let id = self.prog.asserts.len();
        let mut regs = Vec::new();
        for i in 0..n {
            let name = format!("$past{id}_{k}_{i}");
            let full = format!("{}.{name}", self.scopes[sid].name);
            let s = self.new_signal(full, w, signed, 0, w as i64 - 1, 0, 0, SigKind::Reg);
            self.prog.names.pop();
            if two_state {
                self.zero_init(s);
            }
            self.scopes[sid].syms.insert(name.clone(), Sym::Sig(s));
            regs.push(name);
        }
        let last = regs[n as usize - 1].clone();
        b.pasts.push((v.clone(), regs));
        Ok(ExprKind::Ident(last).into())
    }

    /// Whether `e` reads only 2-state variables (its default value is 0, else x).
    fn is_two_state_expr(&self, e: &Expr, sid: usize) -> bool {
        let mut reads = Vec::new();
        self.reads_expr(e, sid, &mut reads);
        !reads.is_empty() && reads.iter().all(|(s, _)| self.sigs.two_state.contains(s))
    }
}

impl<'a> Compiler<'a> {
    /// A deferred immediate assertion (IEEE 1800-2017 16.4): the outcome goes to a hidden bit
    /// and an action process of its own is scheduled for the Reactive (`#0`) or Postponed
    /// (`final`) region; scheduled again before it runs, it reports the latest outcome only
    /// (16.4.2: an earlier report is flushed).
    pub(crate) fn deferred(&mut self, final_: bool, cover: bool, cond: &Expr, pass: &Stmt, fail: &Stmt, sid: usize) -> R<()> {
        let k = self.prog.procs.len();
        let name = format!("$deferred{k}");
        let full = format!("{}.{name}", self.scopes[sid].name);
        let s = self.new_signal(full, 1, false, 0, 0, 0, 0, SigKind::Reg);
        self.prog.names.pop();
        self.scopes[sid].syms.insert(name.clone(), Sym::Sig(s));
        let one = || -> Expr { ExprKind::Num(Number { size: Some(1), signed: false, base: 2, based: true, digits: "1".into(), fill: false }).into() };
        // (x or z is false)
        let ok: Expr = ExprKind::Binary(BinOp::CaseEq, Box::new(ExprKind::Unary(UnOp::RedOr, Box::new(cond.clone())).into()), Box::new(one())).into();
        let line = self.line;
        self.stmt(&Stmt::Assign { lhs: ExprKind::Ident(name.clone()).into(), rhs: ok, nonblocking: false, timing: None, line }, sid)?;
        // `cover`: only a success reports
        if cover && matches!(pass, Stmt::Null) {
            return Ok(());
        }
        let scope = self.scopes[sid].name.clone();
        // (not started at time 0; `Defer` schedules it)
        let p = self.begin_proc(scope, ProcClass::Postponed);
        let act = Stmt::If { cond: ExprKind::Ident(name).into(), then_s: Box::new(pass.clone()), else_s: Some(Box::new(fail.clone())), line };
        let r = self.stmt(&act, sid);
        self.emit(Instr::Halt);
        self.end_proc();
        r?;
        self.emit(Instr::Assert(rt::AssertOp::Defer { proc: p, postponed: final_ }));
        Ok(())
    }
}
