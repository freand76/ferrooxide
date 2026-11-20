//! Concurrent assertions (IEEE 1800-2017 16): sequences as automata over the ticks of the
//! assertion's clock, properties built from them, and the attempts of each assertion.
//!
//! At every tick the compiled code evaluates the assertion's boolean leaves (on sampled
//! values) into a bit vector and steps the assertion: a new attempt starts, the running ones
//! advance. A sequence is an automaton without empty transitions: each transition consumes
//! one tick whose leaves satisfy its condition. An attempt of a property is a tree of
//! instances that ends true or false (or runs on).

/// A literal of a transition's condition: leaf `.0` is true (`.1`) or not true.
pub type Lit = (u32, bool);

/// A sequence as an automaton over ticks.
#[derive(Clone, Debug, Default)]
pub struct Nfa {
    /// the states an attempt starts in (before its first tick)
    pub start: Vec<u32>,
    pub accept: Vec<bool>,
    /// per state: (condition, next state); the condition is a conjunction of literals
    pub trans: Vec<Vec<(Vec<Lit>, u32)>>,
}

impl Nfa {
    pub fn states(&self) -> usize {
        self.accept.len()
    }
}

/// A property of an assertion; children are indexes into `AssertDef::props`.
#[derive(Clone, Debug)]
pub enum PropDef {
    /// holds when the sequence (`AssertDef::nfas` index) matches, fails when it cannot
    Seq(u32),
    Not(u32),
    And(u32, u32),
    Or(u32, u32),
    /// `ante |-> cons` (`next`: `|=>`): for each match of the antecedent sequence, the
    /// consequent property from the tick of the match (or the next one) must hold
    Implies { ante: u32, next: bool, cons: u32 },
    /// `if (leaf) a else b` (no `else`: true)
    If(u32, u32, Option<u32>),
}

/// An assertion: its property (the last of `props`), sequences and kind.
#[derive(Clone, Debug, Default)]
pub struct AssertDef {
    pub nfas: Vec<Nfa>,
    pub props: Vec<PropDef>,
    /// `cover`: only non-vacuous successes count as passes
    pub cover: bool,
    /// `cover sequence`: every match of the sequence of an attempt counts
    pub all_matches: bool,
}

/// The states of an automaton an instance is in.
#[derive(Clone, Debug, Default)]
struct Bits(Vec<u64>);

impl Bits {
    fn new(n: usize) -> Bits {
        Bits(vec![0; n.div_ceil(64)])
    }
    fn set(&mut self, i: u32) {
        self.0[i as usize >> 6] |= 1 << (i & 63);
    }
    fn is_empty(&self) -> bool {
        self.0.iter().all(|&w| w == 0)
    }
    fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().enumerate().flat_map(|(k, &w)| {
            let mut w = w;
            std::iter::from_fn(move || {
                if w == 0 {
                    return None;
                }
                let b = w.trailing_zeros();
                w &= w - 1;
                Some(k as u32 * 64 + b)
            })
        })
    }
}

/// The automaton's states after one tick from `cur` (None: from its start), and whether one
/// of them accepts.
fn advance(nfa: &Nfa, cur: Option<&Bits>, leaves: &dyn Fn(u32) -> bool) -> (Bits, bool) {
    let mut next = Bits::new(nfa.states());
    let mut acc = false;
    let mut from = |s: u32, next: &mut Bits| {
        for (cond, to) in &nfa.trans[s as usize] {
            if cond.iter().all(|&(l, pos)| leaves(l) == pos) {
                // (a state without transitions only matters for its match, now)
                if !nfa.trans[*to as usize].is_empty() {
                    next.set(*to);
                }
                acc |= nfa.accept[*to as usize];
            }
        }
    };
    match cur {
        Some(c) => c.iter().for_each(|s| from(s, &mut next)),
        None => nfa.start.iter().for_each(|&s| from(s, &mut next)),
    }
    (next, acc)
}

/// The result of an instance after a tick: None while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Pass { vacuous: bool },
    Fail,
}

/// A running instance of a property.
#[derive(Clone, Debug)]
enum Inst {
    /// a sequence: its states (None before its first tick)
    Seq { nfa: u32, cur: Option<Bits> },
    Not(Box<Inst>),
    And(Box<Inst>, Box<Inst>, Option<Outcome>, Option<Outcome>),
    Or(Box<Inst>, Box<Inst>, Option<Outcome>, Option<Outcome>),
    Implies {
        ante: u32,
        /// the antecedent's states (None before its first tick); empty: it can match no more
        cur: Option<Bits>,
        next: bool,
        cons: u32,
        /// running consequents
        obl: Vec<Inst>,
        /// the antecedent matched at the last tick (`|=>`: a consequent starts now)
        pending: bool,
        /// a consequent was started (else a success is vacuous)
        started: bool,
    },
    /// `if` before its first tick
    If(u32, u32, Option<u32>),
}

fn inst(def: &AssertDef, p: u32) -> Inst {
    match def.props[p as usize] {
        PropDef::Seq(n) => Inst::Seq { nfa: n, cur: None },
        PropDef::Not(a) => Inst::Not(Box::new(inst(def, a))),
        PropDef::And(a, b) => Inst::And(Box::new(inst(def, a)), Box::new(inst(def, b)), None, None),
        PropDef::Or(a, b) => Inst::Or(Box::new(inst(def, a)), Box::new(inst(def, b)), None, None),
        PropDef::Implies { ante, next, cons } => Inst::Implies { ante, cur: None, next, cons, obl: Vec::new(), pending: false, started: false },
        PropDef::If(c, a, b) => Inst::If(c, a, b),
    }
}

impl Inst {
    fn step(&mut self, def: &AssertDef, leaves: &dyn Fn(u32) -> bool) -> Option<Outcome> {
        match self {
            Inst::Seq { nfa, cur } => {
                let (next, acc) = advance(&def.nfas[*nfa as usize], cur.as_ref(), leaves);
                if acc {
                    return Some(Outcome::Pass { vacuous: false });
                }
                if next.is_empty() {
                    return Some(Outcome::Fail);
                }
                *cur = Some(next);
                None
            }
            Inst::Not(a) => a.step(def, leaves).map(|o| match o {
                Outcome::Pass { .. } => Outcome::Fail,
                Outcome::Fail => Outcome::Pass { vacuous: false },
            }),
            Inst::And(a, b, ra, rb) => {
                if ra.is_none() {
                    *ra = a.step(def, leaves);
                }
                if rb.is_none() {
                    *rb = b.step(def, leaves);
                }
                match (*ra, *rb) {
                    (Some(Outcome::Fail), _) | (_, Some(Outcome::Fail)) => Some(Outcome::Fail),
                    (Some(Outcome::Pass { vacuous: va }), Some(Outcome::Pass { vacuous: vb })) => Some(Outcome::Pass { vacuous: va && vb }),
                    _ => None,
                }
            }
            Inst::Or(a, b, ra, rb) => {
                if ra.is_none() {
                    *ra = a.step(def, leaves);
                }
                if rb.is_none() {
                    *rb = b.step(def, leaves);
                }
                match (*ra, *rb) {
                    (Some(p @ Outcome::Pass { .. }), _) | (_, Some(p @ Outcome::Pass { .. })) => Some(p),
                    (Some(Outcome::Fail), Some(Outcome::Fail)) => Some(Outcome::Fail),
                    _ => None,
                }
            }
            Inst::If(c, a, b) => {
                let chosen = if leaves(*c) { Some(*a) } else { *b };
                match chosen {
                    Some(p) => {
                        *self = inst(def, p);
                        self.step(def, leaves)
                    }
                    None => Some(Outcome::Pass { vacuous: true }),
                }
            }
            Inst::Implies { ante, cur, next, cons, obl, pending, started } => {
                // the consequents started before, then one from a match at the last tick
                let mut failed = false;
                obl.retain_mut(|o| match o.step(def, leaves) {
                    None => true,
                    Some(Outcome::Pass { .. }) => false,
                    Some(Outcome::Fail) => {
                        failed = true;
                        false
                    }
                });
                if std::mem::take(pending) {
                    *started = true;
                    let mut o = inst(def, *cons);
                    match o.step(def, leaves) {
                        None => obl.push(o),
                        Some(Outcome::Fail) => failed = true,
                        Some(Outcome::Pass { .. }) => {}
                    }
                }
                // the antecedent; a match starts a consequent now (`|->`) or at the next tick
                if cur.as_ref().is_none_or(|c| !c.is_empty()) {
                    let (n, acc) = advance(&def.nfas[*ante as usize], cur.as_ref(), leaves);
                    *cur = Some(n);
                    if acc {
                        if *next {
                            *pending = true;
                        } else {
                            *started = true;
                            let mut o = inst(def, *cons);
                            match o.step(def, leaves) {
                                None => obl.push(o),
                                Some(Outcome::Fail) => failed = true,
                                Some(Outcome::Pass { .. }) => {}
                            }
                        }
                    }
                }
                if failed {
                    return Some(Outcome::Fail);
                }
                let ante_done = cur.as_ref().is_some_and(|c| c.is_empty());
                (ante_done && obl.is_empty() && !*pending).then_some(Outcome::Pass { vacuous: !*started })
            }
        }
    }
}

/// The running attempts of one assertion.
#[derive(Clone, Debug, Default)]
pub struct AssertState {
    attempts: Vec<Inst>,
    /// `cover sequence`: the sequences' states per attempt
    seqs: Vec<Bits>,
}

impl AssertState {
    /// One tick: a new attempt starts and all advance. Returns the number of passes and of
    /// failures at this tick.
    pub fn tick(&mut self, def: &AssertDef, leaves: &dyn Fn(u32) -> bool) -> (u32, u32) {
        let (mut pass, mut fail) = (0, 0);
        let top = def.props.len() as u32 - 1;
        if def.all_matches {
            // every match of every attempt
            let PropDef::Seq(n) = def.props[top as usize] else { unreachable!() };
            let nfa = &def.nfas[n as usize];
            let (first, acc) = advance(nfa, None, leaves);
            pass += acc as u32;
            let mut seqs = Vec::with_capacity(self.seqs.len() + 1);
            for s in std::mem::take(&mut self.seqs) {
                let (next, acc) = advance(nfa, Some(&s), leaves);
                pass += acc as u32;
                if !next.is_empty() {
                    seqs.push(next);
                }
            }
            if !first.is_empty() {
                seqs.push(first);
            }
            self.seqs = seqs;
            return (pass, 0);
        }
        self.attempts.push(inst(def, top));
        self.attempts.retain_mut(|a| match a.step(def, leaves) {
            None => true,
            Some(Outcome::Pass { vacuous }) => {
                if !(vacuous && def.cover) {
                    pass += 1;
                }
                false
            }
            Some(Outcome::Fail) => {
                if !def.cover {
                    fail += 1;
                }
                false
            }
        });
        (pass, fail)
    }

    /// `disable iff`: the running attempts end without a result.
    pub fn disable(&mut self) {
        self.attempts.clear();
        self.seqs.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a sequence of one leaf per tick: leaf 0, then leaf 1 at the next tick
    fn two_ticks() -> Nfa {
        Nfa { start: vec![0], accept: vec![false, false, true], trans: vec![vec![(vec![(0, true)], 1)], vec![(vec![(1, true)], 2)], vec![]] }
    }

    fn run(def: &AssertDef, ticks: &[&[bool]]) -> Vec<(u32, u32)> {
        let mut st = AssertState::default();
        ticks.iter().map(|t| st.tick(def, &|l| t[l as usize])).collect()
    }

    #[test]
    fn sequence_property() {
        let def = AssertDef { nfas: vec![two_ticks()], props: vec![PropDef::Seq(0)], ..Default::default() };
        // the attempt of tick 0 passes at tick 1; those of ticks 1 and 2 fail at tick 2
        let r = run(&def, &[&[true, false], &[true, true], &[false, false], &[false, false]]);
        assert_eq!(r, vec![(0, 0), (1, 0), (0, 2), (0, 1)]);
    }

    #[test]
    fn implication() {
        // leaf0 |=> leaf1
        let one = |l| Nfa { start: vec![0], accept: vec![false, true], trans: vec![vec![(vec![(l, true)], 1)], vec![]] };
        let def = AssertDef { nfas: vec![one(0), one(1)], props: vec![PropDef::Seq(1), PropDef::Implies { ante: 0, next: true, cons: 0 }], ..Default::default() };
        let r = run(&def, &[&[false, false], &[true, false], &[false, true], &[true, false], &[false, false]]);
        // tick 0: vacuous pass; 1: a, pending; 2: b holds -> pass, and !a vacuous -> pass;
        // 3: a; 4: b missing -> fail, !a vacuous pass
        assert_eq!(r, vec![(1, 0), (0, 0), (2, 0), (0, 0), (1, 1)]);
    }
}
