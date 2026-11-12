//! The event scheduler (IEEE 1364 stratified regions) and the bytecode interpreter.

use crate::fast::{self, Op};
use crate::ops;
use crate::vcd::Vcd;
use crate::*;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufWriter, Write};
use std::rc::Rc;

#[derive(Clone, Copy, Debug)]
struct SigRt {
    slot: u32,
    width: u32,
    depth: u32,
    /// event fan-out (non-combinational waiters) in `fan`
    fan_lo: u32,
    fan_hi: u32,
    /// combinational readers (comb indices) in `cfan`
    cfan_lo: u32,
    cfan_hi: u32,
    /// for a virtual array-element signal: the array (whose listeners are notified too)
    parent: u32,
}

#[derive(Clone, Copy)]
struct Fan {
    proc: u32,
    ctl: u32,
    edge: Edge,
}

/// Process state. `pc` and `wait` are written directly by JIT code (offsets 0 and 4).
#[derive(Clone, Copy)]
#[repr(C)]
struct ProcRt {
    pc: u32,
    wait: u32,
    class: ProcClass,
    started: bool,
    /// position in rank order in the combinational settle set, NONE if scheduled normally
    rank: u32,
}

struct Code {
    ops: Vec<Vec<Op>>,
    orig: Vec<Vec<Instr>>,
}

/// A resolved store destination inside a signal.
#[derive(Clone, Copy, Debug)]
enum Res {
    Whole,
    Bits { lsb: u32, w: u32, soff: u32 },
    Elem(u32),
    ElemBits { e: u32, lsb: u32, w: u32, soff: u32 },
}

#[derive(Clone, Copy)]
struct Nba {
    sig: u32,
    res: Res,
    /// offset of the value in nba_vals
    off: u32,
    sw: u32,
}

/// An entry of the NBA queue, in scheduling order: a ≤64-bit whole store (`sig`, destination
/// `slot`, value `a`/`b`), or, when `sig == NONE`, a marker for generic entry `slot` of `nba`.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NbaE {
    sig: u32,
    slot: u32,
    a: u64,
    b: u64,
}

/// State that JIT-compiled code reads and writes directly (field offsets are part of the ABI).
#[repr(C)]
pub struct Hot {
    pub nba_ptr: *mut NbaE,
    pub nba_len: u64,
    pub nba_cap: u64,
    pub procs: *mut u8,
}

enum Fut {
    Wake(u32),
    Nba { sig: u32, res: Res, sw: u32, val: Box<[u64]> },
}

#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub activations: u64,
    pub nba_updates: u64,
    pub time_steps: u64,
}

pub struct Sim {
    code: Rc<Code>,
    /// combinational settle set: bit c = combinational process `comb_order[c]` is pending.
    /// Processes are numbered in rank order, so the lowest set bit is the next to evaluate.
    comb_bits: Vec<u64>,
    comb_order: Vec<u32>,
    cfan: Vec<u32>,
    /// (array, element) -> virtual signal of that element
    elem_sig: HashMap<(u32, u32), u32>,
    /// number of real signals (virtual element signals follow)
    nreal: u32,
    /// comb index of the combinational process being executed, NONE otherwise
    cur_comb: u32,
    /// no bits are set below word `comb_lo`
    comb_lo: usize,
    prog: Program,
    pub mem: Vec<u64>,
    sigs: Vec<SigRt>,
    fan: Vec<Fan>,
    procs: Vec<ProcRt>,
    /// active processes with their wake sequence number
    active: VecDeque<(u32, u64)>,
    /// wake sequence counter (FIFO order between active wakes and the combinational settle)
    seq: u64,
    /// sequence number of the first pending combinational wake, u64::MAX if none is pending
    comb_seq: u64,
    inactive: Vec<u32>,
    nba: Vec<Nba>,
    nba_vals: Vec<u64>,
    /// the NBA queue (length in `hot.nba_len`); `nba` holds the generic entries it refers to
    nbaq: Vec<NbaE>,
    hot: Hot,
    future: BTreeMap<u64, Vec<Fut>>,
    postponed: Vec<u32>,
    pub time: u64,
    finished: bool,
    out: BufWriter<std::io::Stdout>,
    monitor: u32,
    monitor_on: bool,
    seed: u32,
    vcd_path: String,
    vcd: Option<Vcd>,
    dumping: bool,
    dirty: Vec<u32>,
    is_dirty: Vec<bool>,
    files: HashMap<u32, BufWriter<std::fs::File>>,
    next_fd: u32,
    scratch: Vec<u64>,
    pub stats: Stats,
    #[cfg(feature = "jit")]
    jit: Option<crate::jit::Jit>,
    /// per process: compiled function and its table (empty when not JIT-compiled)
    #[cfg(feature = "jit")]
    entries: Vec<(crate::jit::ProcFn, *const u32)>,
}

#[inline(always)]
fn lsb_code(mem: &[u64], slot: u32, width: u32) -> u8 {
    let s = slot as usize;
    let n = nw(width);
    ((mem[s] & 1) | ((mem[s + n] & 1) << 1)) as u8
}

/// codes: 0=0, 1=1, 2=z, 3=x
#[inline(always)]
fn edge_fires(edge: Edge, old: u8, new: u8) -> bool {
    match edge {
        Edge::Any => true,
        Edge::Pos => (old == 0 && new != 0) || (old >= 2 && new == 1),
        Edge::Neg => (old == 1 && new != 1) || (old >= 2 && new == 0),
    }
}

impl Sim {
    pub fn new(mut prog: Program) -> Sim {
        if !prog.no_fuse {
            crate::fuse::fuse(&mut prog);
        }
        let nsig = prog.signals.len();
        // build fanout table
        let ranks = comb_ranks(&prog);
        // number the ranked processes in rank order
        let mut comb_order: Vec<u32> = (0..ranks.len() as u32).filter(|&i| ranks[i as usize] != NONE).collect();
        comb_order.sort_by_key(|&i| (ranks[i as usize], i));
        let mut ranks = vec![NONE; ranks.len()];
        for (c, &p) in comb_order.iter().enumerate() {
            ranks[p as usize] = c as u32;
        }
        let comb_words = comb_order.len().div_ceil(64);
        // virtual signals for array elements that something is sensitive to
        let mut elem_sig: HashMap<(u32, u32), u32> = HashMap::new();
        let mut vsig_of: Vec<(u32, u32)> = Vec::new();
        // port aliases of array elements are the elements' signals
        for (i, s) in prog.signals.iter().enumerate() {
            if let Some(ae) = s.alias_of {
                elem_sig.insert(ae, i as u32);
            }
        }
        for c in &prog.ctls {
            for &(sig, _, e) in &c.triggers {
                if e != NONE && !elem_sig.contains_key(&(sig, e)) {
                    elem_sig.insert((sig, e), (nsig + vsig_of.len()) as u32);
                    vsig_of.push((sig, e));
                }
            }
        }
        let ntot = nsig + vsig_of.len();
        // fan-out tables: combinational readers by comb index, everything else as events
        let mut per_sig: Vec<Vec<Fan>> = vec![Vec::new(); ntot];
        let mut per_sig_c: Vec<Vec<u32>> = vec![Vec::new(); ntot];
        for (ci, c) in prog.ctls.iter().enumerate() {
            for &(sig, edge, e) in &c.triggers {
                let sig = if e != NONE { elem_sig[&(sig, e)] } else { sig };
                if c.proc != NONE && ranks[c.proc as usize] != NONE {
                    per_sig_c[sig as usize].push(ranks[c.proc as usize]);
                } else {
                    per_sig[sig as usize].push(Fan { proc: c.proc, ctl: ci as u32, edge });
                }
            }
        }
        let mut fan = Vec::new();
        let mut cfan = Vec::new();
        let mut sigs = Vec::with_capacity(ntot);
        for i in 0..ntot {
            let (slot, width, depth, parent) = if i < nsig {
                let s = &prog.signals[i];
                (s.slot, s.width, s.depth, s.alias_of.map(|a| a.0).unwrap_or(NONE))
            } else {
                let (a, e) = vsig_of[i - nsig];
                let s = &prog.signals[a as usize];
                (s.slot + e * words(s.width), s.width, 0, a)
            };
            let lo = fan.len() as u32;
            fan.extend_from_slice(&per_sig[i]);
            let clo = cfan.len() as u32;
            per_sig_c[i].sort_unstable();
            per_sig_c[i].dedup();
            cfan.extend_from_slice(&per_sig_c[i]);
            sigs.push(SigRt {
                slot,
                width,
                depth,
                fan_lo: lo,
                fan_hi: fan.len() as u32,
                cfan_lo: clo,
                cfan_hi: cfan.len() as u32,
                parent,
            });
        }
        let procs = prog
            .procs
            .iter()
            .enumerate()
            .map(|(i, p)| ProcRt { pc: 0, wait: NONE, class: p.class, started: false, rank: ranks[i] })
            .collect();
        let code = Rc::new(Code {
            ops: prog.procs.iter().map(|p| fast::lower(&p.code, &prog.signals, &elem_sig)).collect(),
            orig: prog.procs.iter().map(|p| p.code.clone()).collect(),
        });
        Sim {
            code,
            comb_bits: vec![0; comb_words],
            comb_order,
            comb_lo: comb_words,
            cfan,
            cur_comb: NONE,
            elem_sig,
            nreal: nsig as u32,
            mem: prog.mem.clone(),
            sigs,
            fan,
            procs,
            active: VecDeque::new(),
            seq: 0,
            comb_seq: u64::MAX,
            inactive: Vec::new(),
            nba: Vec::new(),
            nba_vals: Vec::new(),
            nbaq: Vec::new(),
            hot: Hot { nba_ptr: std::ptr::null_mut(), nba_len: 0, nba_cap: 0, procs: std::ptr::null_mut() },
            future: BTreeMap::new(),
            postponed: Vec::new(),
            time: 0,
            finished: false,
            out: BufWriter::with_capacity(1 << 16, std::io::stdout()),
            monitor: NONE,
            monitor_on: true,
            seed: 0,
            vcd_path: "dump.vcd".into(),
            vcd: None,
            dumping: false,
            dirty: Vec::new(),
            is_dirty: vec![false; nsig],
            files: HashMap::new(),
            next_fd: 3,
            scratch: Vec::new(),
            stats: Stats::default(),
            #[cfg(feature = "jit")]
            jit: None,
            #[cfg(feature = "jit")]
            entries: Vec::new(),
            prog,
        }
    }

    /// Run until no events remain, $finish, or `max_time` is passed.
    pub fn run(&mut self, max_time: Option<u64>) {
        for i in 0..self.procs.len() {
            let p = &mut self.procs[i];
            if p.class == ProcClass::Normal {
                p.started = true;
                if p.rank != NONE {
                    let c = p.rank as usize;
                    self.comb_bits[c >> 6] |= 1 << (c & 63);
                    self.comb_lo = self.comb_lo.min(c >> 6);
                    self.comb_wake();
                } else {
                    self.push_active(i as u32);
                }
            }
        }
        loop {
            self.stats.time_steps += 1;
            self.timestep();
            if self.finished {
                break;
            }
            let Some((t, evs)) = self.future.pop_first() else { break };
            if let Some(mt) = max_time {
                if t > mt {
                    self.time = mt;
                    break;
                }
            }
            self.time = t;
            for ev in evs {
                match ev {
                    Fut::Wake(p) => self.push_active(p),
                    Fut::Nba { sig, res, sw, val } => {
                        let off = self.nba_vals.len() as u32;
                        self.nba_vals.extend_from_slice(&val);
                        self.push_generic(Nba { sig, res, off, sw });
                    }
                }
            }
        }
        // final blocks
        let finals: Vec<u32> =
            (0..self.procs.len() as u32).filter(|&p| self.procs[p as usize].class == ProcClass::Final).collect();
        self.finished = false;
        for p in finals {
            self.exec(p);
        }
        self.flush();
    }

    pub fn flush(&mut self) {
        let _ = self.out.flush();
        if let Some(v) = &mut self.vcd {
            v.flush();
        }
        for f in self.files.values_mut() {
            let _ = f.flush();
        }
    }

    fn timestep(&mut self) {
        loop {
            // Combinational processes settle before the next active process runs: continuous
            // assignments propagate "immediately", as with Icarus's FIFO order in the common
            // `data = x; clk = 1;` testbench race, and at time 0 every combinational process
            // reaches its first wait before initial blocks change its inputs.
            let front = self.active.front().map(|a| a.1).unwrap_or(u64::MAX);
            if self.comb_seq < front {
                if self.settle() {
                    return;
                }
                continue;
            }
            if let Some((p, _)) = self.active.pop_front() {
                self.exec(p);
                if self.finished {
                    return;
                }
                continue;
            }
            if !self.inactive.is_empty() {
                for p in std::mem::take(&mut self.inactive) {
                    self.push_active(p);
                }
                continue;
            }
            if self.hot.nba_len != 0 {
                self.apply_nba();
                continue;
            }
            break;
        }
        if !self.postponed.is_empty() {
            let pp = std::mem::take(&mut self.postponed);
            for p in pp {
                self.exec(p);
            }
        }
        if self.dumping && !self.dirty.is_empty() {
            self.dump_dirty();
        }
    }

    #[inline]
    fn push_active(&mut self, p: u32) {
        self.active.push_back((p, self.seq));
        self.seq += 1;
    }

    /// A combinational process became pending: the settle is ordered by its first wake.
    #[inline]
    fn comb_wake(&mut self) {
        if self.comb_seq == u64::MAX {
            self.comb_seq = self.seq;
            self.seq += 1;
        }
    }

    /// Run pending combinational processes in rank order. Returns true on $finish.
    #[inline]
    fn settle(&mut self) -> bool {
        loop {
            while self.comb_lo < self.comb_bits.len() && self.comb_bits[self.comb_lo] == 0 {
                self.comb_lo += 1;
            }
            if self.comb_lo >= self.comb_bits.len() {
                self.comb_seq = u64::MAX;
                return false;
            }
            let w = self.comb_bits[self.comb_lo];
            let bit = w.trailing_zeros() as usize;
            self.comb_bits[self.comb_lo] = w & (w - 1);
            let c = (self.comb_lo << 6) | bit;
            let k = self.comb_order[c];
            self.cur_comb = c as u32;
            self.exec(k);
            self.cur_comb = NONE;
            if self.finished {
                return true;
            }
        }
    }

    fn dump_dirty(&mut self) {
        let vcd = self.vcd.as_mut().unwrap();
        vcd.time(self.time);
        for &s in &self.dirty {
            let r = self.sigs[s as usize];
            vcd.write_value(s, &self.mem, r.slot, r.width);
            self.is_dirty[s as usize] = false;
        }
        self.dirty.clear();
    }

    #[inline]
    fn push_small(&mut self, sig: u32, slot: u32, a: u64, b: u64) {
        if self.hot.nba_len == self.hot.nba_cap {
            unsafe { self.nbaq.set_len(self.hot.nba_len as usize) };
            self.nbaq.reserve(self.nbaq.len().max(256));
            self.hot.nba_ptr = self.nbaq.as_mut_ptr();
            self.hot.nba_cap = self.nbaq.capacity() as u64;
        }
        // SAFETY: nba_len < nba_cap, the buffer belongs to `nbaq`
        unsafe { *self.hot.nba_ptr.add(self.hot.nba_len as usize) = NbaE { sig, slot, a, b } };
        self.hot.nba_len += 1;
    }

    fn push_generic(&mut self, e: Nba) {
        let i = self.nba.len() as u32;
        self.nba.push(e);
        self.push_small(NONE, i, 0, 0);
    }

    fn apply_nba(&mut self) {
        unsafe { self.nbaq.set_len(self.hot.nba_len as usize) };
        let q = std::mem::take(&mut self.nbaq);
        self.hot.nba_len = 0;
        self.hot.nba_ptr = std::ptr::null_mut();
        self.hot.nba_cap = 0;
        let list = std::mem::take(&mut self.nba);
        let vals = std::mem::take(&mut self.nba_vals);
        self.stats.nba_updates += q.len() as u64;
        for e in &q {
            if e.sig != NONE {
                let d = e.slot as usize;
                if self.mem[d] != e.a || self.mem[d + 1] != e.b {
                    let old = ((self.mem[d] & 1) | ((self.mem[d + 1] & 1) << 1)) as u8;
                    self.mem[d] = e.a;
                    self.mem[d + 1] = e.b;
                    self.notify(e.sig, old);
                }
                continue;
            }
            let g = &list[e.slot as usize];
            let n = 2 * nw(g.sw);
            let o = g.off as usize;
            self.apply(g.sig, g.res, &vals[o..o + n], g.sw);
        }
        // reuse the queue's buffer (nothing is scheduled while NBAs are applied)
        let mut q = q;
        q.clear();
        if self.hot.nba_len == 0 && self.nbaq.capacity() == 0 {
            self.hot.nba_ptr = q.as_mut_ptr();
            self.hot.nba_cap = q.capacity() as u64;
            self.nbaq = q;
        }
        // reuse allocations
        let mut list = list;
        list.clear();
        let mut vals = vals;
        vals.clear();
        if self.nba.is_empty() {
            self.nba = list;
        }
        if self.nba_vals.is_empty() {
            self.nba_vals = vals;
        }
    }

    #[inline]
    fn notify(&mut self, sig: u32, old: u8) {
        let r = self.sigs[sig as usize];
        if r.parent != NONE {
            self.notify(r.parent, 0);
        }
        if self.dumping && sig < self.nreal && !self.is_dirty[sig as usize] {
            self.is_dirty[sig as usize] = true;
            self.dirty.push(sig);
        }
        for i in r.cfan_lo..r.cfan_hi {
            // a combinational process is either waiting on its inputs or running right now
            let c = unsafe { *self.cfan.get_unchecked(i as usize) };
            if c != self.cur_comb {
                let wi = (c >> 6) as usize;
                self.comb_bits[wi] |= 1 << (c & 63);
                self.comb_wake();
                if wi < self.comb_lo {
                    self.comb_lo = wi;
                }
            }
        }
        if r.fan_lo == r.fan_hi {
            return;
        }
        let new = if r.depth == 0 { lsb_code(&self.mem, r.slot, r.width) } else { 0 };
        for i in r.fan_lo..r.fan_hi {
            let f = self.fan[i as usize];
            let pr = &mut self.procs[f.proc as usize];
            if pr.wait != f.ctl {
                continue;
            }
            if r.depth == 0 && !edge_fires(f.edge, old, new) {
                continue;
            }
            pr.wait = NONE;
            if pr.rank != NONE {
                let c = pr.rank as usize;
                self.comb_bits[c >> 6] |= 1 << (c & 63);
                self.comb_wake();
                if (c >> 6) < self.comb_lo {
                    self.comb_lo = c >> 6;
                }
            } else if pr.class == ProcClass::Postponed {
                self.postponed.push(f.proc);
            } else {
                self.push_active(f.proc);
            }
        }
    }

    /// Write `src` (planes of a `sw`-bit value) into a signal, notifying on change.
    fn apply(&mut self, sig: u32, res: Res, src: &[u64], sw: u32) {
        let r = self.sigs[sig as usize];
        let n = nw(r.width);
        let ns = nw(sw);
        let mut base = r.slot as usize;
        let old = if r.depth == 0 { lsb_code(&self.mem, r.slot, r.width) } else { 0 };
        let changed = match res {
            Res::Whole | Res::Elem(_) => {
                if let Res::Elem(e) = res {
                    base += e as usize * 2 * n;
                }
                let dst = &mut self.mem[base..base + 2 * n];
                if dst == &src[..2 * n] {
                    false
                } else {
                    dst.copy_from_slice(&src[..2 * n]);
                    true
                }
            }
            Res::Bits { lsb, w, soff } | Res::ElemBits { lsb, w, soff, .. } => {
                if let Res::ElemBits { e, .. } = res {
                    base += e as usize * 2 * n;
                }
                let mut changed = false;
                for plane in 0..2 {
                    let mut i = 0;
                    while i < w {
                        let len = (w - i).min(64);
                        let v = ops::get_bits(src, plane * ns, soff + i, len);
                        let o = ops::get_bits(&self.mem, base + plane * n, lsb + i, len);
                        if v != o {
                            ops::set_bits(&mut self.mem, base + plane * n, lsb + i, len, v);
                            changed = true;
                        }
                        i += len;
                    }
                }
                changed
            }
        };
        if changed {
            let e = match res {
                Res::Elem(e) | Res::ElemBits { e, .. } => e,
                _ => NONE,
            };
            match (e != NONE).then(|| self.elem_sig.get(&(sig, e)).copied()).flatten() {
                Some(v) => {
                    let old = lsb_code(&self.mem, self.sigs[v as usize].slot, r.width);
                    let _ = old;
                    self.notify(v, 0);
                }
                None => self.notify(sig, old),
            }
        }
    }

    #[inline]
    fn index(&self, ix: &Index) -> Option<i64> {
        let v = ops::read_index(&self.mem, ix.slot as usize, ix.w, ix.signed)?;
        Some(if ix.neg { ix.k.saturating_sub(v) } else { v.saturating_add(ix.k) })
    }

    /// Resolve a target to (signal, destination, source width).
    fn resolve(&self, t: &Target) -> Option<(u32, Res, u32)> {
        match *t {
            Target::Whole { sig } => Some((sig, Res::Whole, self.sigs[sig as usize].width)),
            Target::Part { sig, lsb, w } => Some((sig, Res::Bits { lsb, w, soff: 0 }, w)),
            Target::DynPart { sig, ix, w } => {
                let off = self.index(&ix)?;
                let width = self.sigs[sig as usize].width as i64;
                let lo = off.max(0);
                let hi = (off + w as i64).min(width);
                if lo >= hi {
                    return None;
                }
                Some((sig, Res::Bits { lsb: lo as u32, w: (hi - lo) as u32, soff: (lo - off) as u32 }, w))
            }
            Target::Elem { sig, ix } => {
                let e = self.index(&ix)?;
                let r = &self.sigs[sig as usize];
                if e < 0 || e >= r.depth as i64 {
                    return None;
                }
                Some((sig, Res::Elem(e as u32), r.width))
            }
            Target::ElemK { sig, e } => Some((sig, Res::Elem(e), self.sigs[sig as usize].width)),
            Target::ElemPart { sig, ix, lsb, w } => {
                let e = self.index(&ix)?;
                let r = &self.sigs[sig as usize];
                if e < 0 || e >= r.depth as i64 {
                    return None;
                }
                Some((sig, Res::ElemBits { e: e as u32, lsb, w, soff: 0 }, w))
            }
        }
    }

    fn store(&mut self, t: &Target, s: u32, mode: StoreMode) {
        let Some((sig, res, sw)) = self.resolve(t) else { return };
        let n = 2 * nw(sw);
        let s = s as usize;
        match mode {
            StoreMode::Blocking => {
                if n == 2 {
                    let tmp = [self.mem[s], self.mem[s + 1]];
                    self.apply(sig, res, &tmp, sw);
                } else {
                    let mut tmp = std::mem::take(&mut self.scratch);
                    tmp.clear();
                    tmp.extend_from_slice(&self.mem[s..s + n]);
                    self.apply(sig, res, &tmp, sw);
                    self.scratch = tmp;
                }
            }
            StoreMode::Nba => {
                if n == 2 && matches!(res, Res::Whole) {
                    let slot = self.sigs[sig as usize].slot;
                    let (a, b) = (self.mem[s], self.mem[s + 1]);
                    self.push_small(sig, slot, a, b);
                    return;
                }
                let off = self.nba_vals.len() as u32;
                self.nba_vals.extend_from_slice(&self.mem[s..s + n]);
                self.push_generic(Nba { sig, res, off, sw });
            }
            StoreMode::NbaDelay(ds) => {
                let d = ops::read_u64(&self.mem, ds as usize, 64).unwrap_or(0);
                let val: Box<[u64]> = self.mem[s..s + n].into();
                if d == 0 {
                    let off = self.nba_vals.len() as u32;
                    self.nba_vals.extend_from_slice(&val);
                    self.push_generic(Nba { sig, res, off, sw });
                } else {
                    self.future.entry(self.time + d).or_default().push(Fut::Nba { sig, res, sw, val });
                }
            }
        }
    }

    /// Compile all processes to native code (no-op without the `jit` feature).
    pub fn enable_jit(&mut self) -> Result<(), String> {
        #[cfg(feature = "jit")]
        {
            let cb = crate::jit::Callbacks {
                notify: cb_notify,
                nba: cb_nba,
                nba_wide: cb_nba_wide,
                gen: cb_gen,
                wait: cb_wait,
                delay: cb_delay,
                halt: cb_halt,
            };
            let j = crate::jit::compile(
                &self.code.ops,
                &self.code.orig,
                self.prog.scratch_words,
                &cb,
                std::mem::size_of::<ProcRt>() as u32,
            )?;
            self.entries = (0..j.fns.len()).map(|p| (j.fns[p], j.tables[j.table_off[p]..].as_ptr())).collect();
            self.jit = Some(j);
        }
        Ok(())
    }

    /// Execute process `p` until it suspends.
    fn exec(&mut self, p: u32) {
        self.stats.activations += 1;
        #[cfg(feature = "jit")]
        if !self.entries.is_empty() {
            let (f, table) = unsafe { *self.entries.get_unchecked(p as usize) };
            let pc = self.procs[p as usize].pc;
            let mem = self.mem.as_mut_ptr();
            self.hot.procs = self.procs.as_mut_ptr() as *mut u8;
            let hot = &mut self.hot as *mut Hot;
            // SAFETY: generated code only touches `mem` at compiler-allocated slots, its own
            // process record, the NBA queue through `hot`, and calls back into cb_* below.
            unsafe { f(self as *mut Sim, mem, pc, table, hot) };
            return;
        }
        // SAFETY: the code is never modified while the simulation runs, and `self.code` outlives
        // this call; avoiding the Rc clone keeps a refcount update off every activation.
        let code: &Code = unsafe { &*Rc::as_ptr(&self.code) };
        let ops = &code.ops[p as usize][..];
        let orig = &code.orig[p as usize][..];
        let mut pc = self.procs[p as usize].pc as usize;
        // SAFETY: slots are allocated by the compiler inside `mem`, which never shrinks.
        macro_rules! m {
            ($i:expr) => {
                *unsafe { self.mem.get_unchecked_mut($i as usize) }
            };
        }
        loop {
            let op = unsafe { *ops.get_unchecked(pc) };
            pc += 1;
            match op {
                // the interpreter runs wide ops through the generic path (the JIT compiles them)
                Op::Gen | Op::GenJ | Op::WCopy { .. } | Op::WAnd { .. } | Op::WOr { .. } | Op::WXor { .. } | Op::WNot { .. } | Op::WAdd { .. } | Op::WSub { .. } | Op::WEq { .. } | Op::WNe { .. } | Op::WCaseEq { .. } | Op::WCaseNe { .. } | Op::WLt { .. } | Op::WLe { .. } | Op::WSlice { .. } | Op::WResize { .. } | Op::WIns { .. } | Op::WMux { .. } | Op::WBool { .. } | Op::WStore { .. } | Op::WNba { .. } => {
                    if self.exec_gen(p, &orig[pc - 1], &mut pc) {
                        return;
                    }
                }
                Op::Copy1 { d, s } => {
                    m!(d) = m!(s);
                    m!(d + 1) = m!(s + 1);
                }
                Op::Ins1 { d, s, off, w } => {
                    let mask = fast::mk(w) << off;
                    m!(d) = (m!(d) & !mask) | ((m!(s) << off) & mask);
                    m!(d + 1) = (m!(d + 1) & !mask) | ((m!(s + 1) << off) & mask);
                }
                Op::Slice1 { d, s, sh, w } => {
                    m!(d) = (m!(s) >> sh) & fast::mk(w);
                    m!(d + 1) = (m!(s + 1) >> sh) & fast::mk(w);
                }
                Op::Trunc1 { d, s, w } => {
                    let mask = fast::mk(w);
                    m!(d) = m!(s) & mask;
                    m!(d + 1) = m!(s + 1) & mask;
                }
                Op::Sext1 { d, s, sw, w } => {
                    let mask = fast::mk(w);
                    m!(d) = fast::sext(m!(s), sw as u32) & mask;
                    m!(d + 1) = fast::sext(m!(s + 1), sw as u32) & mask;
                }
                Op::Not1 { d, s, w } => {
                    let mask = fast::mk(w);
                    let (a, b) = (m!(s), m!(s + 1));
                    m!(d) = (!a | b) & mask;
                    m!(d + 1) = b;
                }
                Op::And1 { d, a, b } => {
                    let (r, x) = fast::and1(m!(a), m!(a + 1), m!(b), m!(b + 1));
                    m!(d) = r;
                    m!(d + 1) = x;
                }
                Op::Or1 { d, a, b } => {
                    let (r, x) = fast::or1(m!(a), m!(a + 1), m!(b), m!(b + 1));
                    m!(d) = r;
                    m!(d + 1) = x;
                }
                Op::Xor1 { d, a, b } => {
                    let x = m!(a + 1) | m!(b + 1);
                    m!(d) = (m!(a) ^ m!(b)) | x;
                    m!(d + 1) = x;
                }
                Op::Xnor1 { d, a, b, w } => {
                    let mask = fast::mk(w);
                    let x = m!(a + 1) | m!(b + 1);
                    m!(d) = (!(m!(a) ^ m!(b)) | x) & mask;
                    m!(d + 1) = x;
                }
                Op::Add1 { d, a, b, w } => {
                    let mask = fast::mk(w);
                    if (m!(a + 1) | m!(b + 1)) == 0 {
                        m!(d) = m!(a).wrapping_add(m!(b)) & mask;
                        m!(d + 1) = 0;
                    } else {
                        m!(d) = mask;
                        m!(d + 1) = mask;
                    }
                }
                Op::Sub1 { d, a, b, w } => {
                    let mask = fast::mk(w);
                    if (m!(a + 1) | m!(b + 1)) == 0 {
                        m!(d) = m!(a).wrapping_sub(m!(b)) & mask;
                        m!(d + 1) = 0;
                    } else {
                        m!(d) = mask;
                        m!(d + 1) = mask;
                    }
                }
                Op::Mul1 { d, a, b, w } => {
                    let mask = fast::mk(w);
                    if (m!(a + 1) | m!(b + 1)) == 0 {
                        m!(d) = m!(a).wrapping_mul(m!(b)) & mask;
                        m!(d + 1) = 0;
                    } else {
                        m!(d) = mask;
                        m!(d + 1) = mask;
                    }
                }
                Op::Eq1 { d, a, b } | Op::Ne1 { d, a, b } => {
                    let (va, xa, vb, xb) = (m!(a), m!(a + 1), m!(b), m!(b + 1));
                    let ne = matches!(op, Op::Ne1 { .. }) as u64;
                    if (xa | xb) == 0 {
                        m!(d) = ((va == vb) as u64) ^ ne;
                        m!(d + 1) = 0;
                    } else if (va ^ vb) & !xa & !xb != 0 {
                        m!(d) = ne;
                        m!(d + 1) = 0;
                    } else {
                        m!(d) = 1;
                        m!(d + 1) = 1;
                    }
                }
                Op::CaseEq1 { d, a, b } => {
                    m!(d) = (m!(a) == m!(b) && m!(a + 1) == m!(b + 1)) as u64;
                    m!(d + 1) = 0;
                }
                Op::CaseNe1 { d, a, b } => {
                    m!(d) = !(m!(a) == m!(b) && m!(a + 1) == m!(b + 1)) as u64;
                    m!(d + 1) = 0;
                }
                Op::Lt1 { d, a, b } => {
                    if (m!(a + 1) | m!(b + 1)) == 0 {
                        m!(d) = (m!(a) < m!(b)) as u64;
                        m!(d + 1) = 0;
                    } else {
                        m!(d) = 1;
                        m!(d + 1) = 1;
                    }
                }
                Op::Le1 { d, a, b } => {
                    if (m!(a + 1) | m!(b + 1)) == 0 {
                        m!(d) = (m!(a) <= m!(b)) as u64;
                        m!(d + 1) = 0;
                    } else {
                        m!(d) = 1;
                        m!(d + 1) = 1;
                    }
                }
                Op::Shl1 { d, a, b, w } | Op::Shr1 { d, a, b, w } => {
                    let mask = fast::mk(w);
                    if m!(b + 1) != 0 {
                        m!(d) = mask;
                        m!(d + 1) = mask;
                    } else {
                        let k = m!(b);
                        if k >= w as u64 {
                            m!(d) = 0;
                            m!(d + 1) = 0;
                        } else if matches!(op, Op::Shl1 { .. }) {
                            m!(d) = (m!(a) << k) & mask;
                            m!(d + 1) = (m!(a + 1) << k) & mask;
                        } else {
                            m!(d) = m!(a) >> k;
                            m!(d + 1) = m!(a + 1) >> k;
                        }
                    }
                }
                Op::Cat2 { d, hi, lo, sh } => {
                    m!(d) = (m!(hi) << sh) | m!(lo);
                    m!(d + 1) = (m!(hi + 1) << sh) | m!(lo + 1);
                }
                Op::Mux1 { d, c, t, f } => {
                    let (ca, cb) = (m!(c), m!(c + 1));
                    if ca & !cb != 0 {
                        m!(d) = m!(t);
                        m!(d + 1) = m!(t + 1);
                    } else if cb == 0 {
                        m!(d) = m!(f);
                        m!(d + 1) = m!(f + 1);
                    } else {
                        let (ta, tb, fa, fb) = (m!(t), m!(t + 1), m!(f), m!(f + 1));
                        let x = tb | fb | (ta ^ fa);
                        m!(d) = ta | x;
                        m!(d + 1) = x;
                    }
                }
                Op::Bool1 { d, s } => {
                    let (a, b) = (m!(s), m!(s + 1));
                    if a & !b != 0 {
                        m!(d) = 1;
                        m!(d + 1) = 0;
                    } else if b == 0 {
                        m!(d) = 0;
                        m!(d + 1) = 0;
                    } else {
                        m!(d) = 1;
                        m!(d + 1) = 1;
                    }
                }
                Op::RedAnd1 { d, s, w } => {
                    let mask = fast::mk(w);
                    let (a, b) = (m!(s), m!(s + 1));
                    if (!a & !b & mask) != 0 {
                        m!(d) = 0;
                        m!(d + 1) = 0;
                    } else if b != 0 {
                        m!(d) = 1;
                        m!(d + 1) = 1;
                    } else {
                        m!(d) = 1;
                        m!(d + 1) = 0;
                    }
                }
                Op::RedXor1 { d, s } => {
                    let (a, b) = (m!(s), m!(s + 1));
                    if b != 0 {
                        m!(d) = 1;
                        m!(d + 1) = 1;
                    } else {
                        m!(d) = (a.count_ones() & 1) as u64;
                        m!(d + 1) = 0;
                    }
                }
                Op::Jmp { to } => pc = to as usize,
                Op::Jz1 { c, to } => {
                    if m!(c) & !m!(c + 1) == 0 {
                        pc = to as usize;
                    }
                }
                Op::Store1 { sig, s, slot } => {
                    let (a, b) = (m!(s), m!(s + 1));
                    let (oa, ob) = (m!(slot), m!(slot + 1));
                    if oa != a || ob != b {
                        m!(slot) = a;
                        m!(slot + 1) = b;
                        self.notify(sig, ((oa & 1) | ((ob & 1) << 1)) as u8);
                    }
                }
                Op::Wait { ctl } => {
                    let pr = &mut self.procs[p as usize];
                    pr.wait = ctl;
                    pr.pc = pc as u32;
                    return;
                }
                Op::Delay { s } => {
                    let d = if m!(s + 1) == 0 { m!(s) } else { 0 };
                    if d == 0 {
                        self.inactive.push(p);
                    } else {
                        self.future.entry(self.time + d).or_default().push(Fut::Wake(p));
                    }
                    self.procs[p as usize].pc = pc as u32;
                    return;
                }
                Op::Halt => {
                    let pr = &mut self.procs[p as usize];
                    pr.pc = (pc - 1) as u32;
                    pr.wait = NONE;
                    return;
                }
                Op::Nba1 { sig, s, slot } => {
                    let (a, b) = (m!(s), m!(s + 1));
                    self.push_small(sig, slot, a, b);
                }
            }
        }
    }

    /// Execute one instruction through the generic path. `pc` already points past it.
    /// Returns true if the process suspended.
    #[inline(never)]
    fn exec_gen(&mut self, p: u32, ins: &Instr, pc: &mut usize) -> bool {
        {
            let m = &mut self.mem;
            match ins {
                Instr::Copy { d, s, w } => ops::copy(m, *d as usize, *s as usize, *w),
                Instr::Resize { d, s, sw, dw, sign } => ops::resize(m, *d as usize, *s as usize, *sw, *dw, *sign),
                Instr::Un { op, d, s, w } => ops::unop(m, *op, *d as usize, *s as usize, *w),
                Instr::Red { op, d, s, w } => ops::redop(m, *op, *d as usize, *s as usize, *w),
                Instr::Bin { op, d, a, b, w } => ops::binop(m, *op, *d as usize, *a as usize, *b as usize, *w),
                Instr::Cmp { op, d, a, b, w } => ops::cmpop(m, *op, *d as usize, *a as usize, *b as usize, *w),
                Instr::Shift { op, d, a, w, b, bw } => {
                    ops::shift(m, *op, *d as usize, *a as usize, *w, *b as usize, *bw)
                }
                Instr::Pow { d, a, b, w, bw, signed } => {
                    ops::pow(m, *d as usize, *a as usize, *b as usize, *w, *bw, *signed)
                }
                Instr::Mux { d, c, cw, t, f, w } => {
                    ops::mux(m, *d as usize, *c as usize, *cw, *t as usize, *f as usize, *w)
                }
                Instr::Concat { d, w, parts } => ops::concat(m, *d as usize, *w, parts),
                Instr::Insert { d, dw, s, sw, off } => ops::insert(m, *d as usize, *dw, *s as usize, *sw, *off),
                Instr::Slice { d, s, sw, lsb, w } => ops::extract(m, *d as usize, *s as usize, *sw, *lsb as i64, *w),
                Instr::DynSlice { d, s, sw, ix, w } => match self.index(ix) {
                    Some(off) => ops::extract(&mut self.mem, *d as usize, *s as usize, *sw, off, *w),
                    None => ops::set_x(&mut self.mem, *d as usize, *w),
                },
                Instr::ArrRead { d, sig, ix } => {
                    let r = self.sigs[*sig as usize];
                    match self.index(ix) {
                        Some(e) if e >= 0 && e < r.depth as i64 => {
                            let n = 2 * nw(r.width);
                            let src = r.slot as usize + e as usize * n;
                            self.mem.copy_within(src..src + n, *d as usize);
                        }
                        _ => ops::set_x(&mut self.mem, *d as usize, r.width),
                    }
                }
                Instr::Store { t, s, mode } => {
                    // fast path: blocking whole-signal store of a ≤64-bit value
                    if let (Target::Whole { sig }, StoreMode::Blocking) = (t, mode) {
                        let r = self.sigs[*sig as usize];
                        if r.width <= 64 {
                            let (s, d) = (*s as usize, r.slot as usize);
                            let (a, b) = (self.mem[s], self.mem[s + 1]);
                            if self.mem[d] != a || self.mem[d + 1] != b {
                                let old = ((self.mem[d] & 1) | ((self.mem[d + 1] & 1) << 1)) as u8;
                                self.mem[d] = a;
                                self.mem[d + 1] = b;
                                self.notify(*sig, old);
                            }
                            return false;
                        }
                    }
                    self.store(t, *s, *mode);
                }
                Instr::Jmp { to } => *pc = *to as usize,
                Instr::Jz { c, w, to } => {
                    if ops::truth(m, *c as usize, *w) != Some(true) {
                        *pc = *to as usize;
                    }
                }
                Instr::Delay { s } => {
                    let d = ops::read_u64(m, *s as usize, 64).unwrap_or(0);
                    if d == 0 {
                        self.inactive.push(p);
                    } else {
                        self.future.entry(self.time + d).or_default().push(Fut::Wake(p));
                    }
                    self.procs[p as usize].pc = *pc as u32;
                    return true;
                }
                Instr::Wait { ctl } => {
                    let pr = &mut self.procs[p as usize];
                    pr.wait = *ctl;
                    pr.pc = *pc as u32;
                    return true;
                }
                Instr::Halt => {
                    let pr = &mut self.procs[p as usize];
                    pr.pc = (*pc - 1) as u32;
                    pr.wait = NONE;
                    return true;
                }
                Instr::SysF { f, d } => {
                    let d = *d as usize;
                    match f {
                        SysFn::Time => {
                            m[d] = self.time;
                            m[d + 1] = 0;
                        }
                        SysFn::Random | SysFn::Urandom => {
                            let v = self.next_random();
                            self.mem[d] = v as u64;
                            self.mem[d + 1] = 0;
                        }
                        SysFn::RandomSeed(sig) => {
                            let r = self.sigs[*sig as usize];
                            let seed = self.mem[r.slot as usize] as u32;
                            let (v, ns) = random_step(seed);
                            self.mem[d] = v as u64;
                            self.mem[d + 1] = 0;
                            let tmp = [ns as u64 & ops::mask(r.width), 0];
                            self.apply(*sig, Res::Whole, &tmp[..2.min(2 * nw(r.width))], r.width.min(64));
                        }
                    }
                }
                Instr::Clog2 { d, s, w } => ops::clog2(m, *d as usize, *s as usize, *w),
                Instr::Sys(task) => {
                    if self.systask(p, task, *pc) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn next_random(&mut self) -> u32 {
        let (v, ns) = random_step(self.seed);
        self.seed = ns;
        v
    }

    /// Returns true if the process must stop executing.
    fn systask(&mut self, p: u32, task: &SysTask, pc: usize) -> bool {
        match task {
            SysTask::Display { nl, args, scope, fd, prefix } => {
                let mut s = String::from(*prefix);
                s.push_str(&fmt::format_args(args, &self.mem, scope, 'd'));
                if *nl {
                    s.push('\n');
                }
                match fd {
                    None => {
                        let _ = self.out.write_all(s.as_bytes());
                    }
                    Some((slot, w)) => {
                        let v = ops::read_u64(&self.mem, *slot as usize, *w).unwrap_or(0) as u32;
                        if v & 0x8000_0000 == 0 {
                            // multichannel descriptor: bit0 = stdout
                            if v & 1 != 0 {
                                let _ = self.out.write_all(s.as_bytes());
                            }
                            if v & 2 != 0 {
                                eprint!("{s}");
                            }
                        } else if let Some(f) = self.files.get_mut(&v) {
                            let _ = f.write_all(s.as_bytes());
                        }
                    }
                }
            }
            SysTask::Monitor { proc } => {
                self.monitor = *proc;
                let pr = &mut self.procs[*proc as usize];
                if !pr.started {
                    pr.started = true;
                    self.postponed.push(*proc);
                }
            }
            SysTask::MonitorPrint { proc, args, scope } => {
                if self.monitor == *proc && self.monitor_on {
                    let mut s = fmt::format_args(args, &self.mem, scope, 'd');
                    s.push('\n');
                    let _ = self.out.write_all(s.as_bytes());
                }
            }
            SysTask::MonitorOn(on) => self.monitor_on = *on,
            SysTask::Strobe { proc } => {
                self.procs[*proc as usize].pc = 0;
                self.postponed.push(*proc);
            }
            SysTask::Finish => {
                self.finished = true;
                self.procs[p as usize].pc = pc as u32;
                self.flush();
                return true;
            }
            SysTask::DumpFile(f) => self.vcd_path = f.clone(),
            SysTask::DumpVars => {
                if self.vcd.is_none() {
                    match Vcd::create(&self.vcd_path, &self.prog) {
                        Ok(mut v) => {
                            v.time(self.time);
                            v.begin_dumpvars();
                            for (i, r) in self.sigs.iter().enumerate() {
                                if r.depth == 0 {
                                    v.write_value(i as u32, &self.mem, r.slot, r.width);
                                }
                            }
                            v.end_dumpvars();
                            self.vcd = Some(v);
                            self.dumping = true;
                        }
                        Err(e) => eprintln!("warning: cannot create VCD file {}: {e}", self.vcd_path),
                    }
                }
            }
            SysTask::DumpOn(on) => {
                if self.vcd.is_some() {
                    self.dumping = *on;
                }
            }
            SysTask::ReadMem { hex, file, sig, start, end } => {
                let start = start.and_then(|(s, w)| ops::read_u64(&self.mem, s as usize, w));
                let end = end.and_then(|(s, w)| ops::read_u64(&self.mem, s as usize, w));
                self.readmem(*hex, file, *sig, start, end);
            }
            SysTask::FOpen { d, dw, file } => {
                let fd = match std::fs::File::create(file) {
                    Ok(f) => {
                        let fd = 0x8000_0000 | self.next_fd;
                        self.next_fd += 1;
                        self.files.insert(fd, BufWriter::new(f));
                        fd
                    }
                    Err(_) => 0,
                };
                let tmp = [fd as u64 & ops::mask(*dw), 0];
                let d = *d as usize;
                self.mem[d] = tmp[0];
                self.mem[d + 1] = 0;
            }
            SysTask::FClose { s } => {
                let v = self.mem[*s as usize] as u32;
                if let Some(mut f) = self.files.remove(&v) {
                    let _ = f.flush();
                }
            }
        }
        false
    }

    fn readmem(&mut self, hex: bool, file: &str, sig: u32, start: Option<u64>, end: Option<u64>) {
        let text = match std::fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("warning: $readmem: cannot open '{file}': {e}");
                return;
            }
        };
        let info = self.prog.signals[sig as usize].clone();
        let r = self.sigs[sig as usize];
        let lo = info.arr_lo;
        let mut addr: i64 = start.map(|s| s as i64).unwrap_or(lo);
        let last: i64 = end.map(|e| e as i64).unwrap_or(lo + r.depth as i64 - 1);
        // strip comments
        let mut clean = String::new();
        let mut it = text.chars().peekable();
        while let Some(c) = it.next() {
            if c == '/' && it.peek() == Some(&'/') {
                for c2 in it.by_ref() {
                    if c2 == '\n' {
                        break;
                    }
                }
                clean.push(' ');
            } else if c == '/' && it.peek() == Some(&'*') {
                it.next();
                let mut prev = ' ';
                for c2 in it.by_ref() {
                    if prev == '*' && c2 == '/' {
                        break;
                    }
                    prev = c2;
                }
                clean.push(' ');
            } else {
                clean.push(c);
            }
        }
        let n = nw(r.width);
        for tok in clean.split_whitespace() {
            if let Some(a) = tok.strip_prefix('@') {
                addr = i64::from_str_radix(a, 16).unwrap_or(addr);
                continue;
            }
            if addr > last {
                break;
            }
            let e = addr - lo;
            if e >= 0 && (e as u32) < r.depth {
                let mut a = vec![0u64; n];
                let mut b = vec![0u64; n];
                let bits = if hex { 4 } else { 1 };
                let mut pos = 0u32;
                for ch in tok.chars().rev() {
                    if ch == '_' {
                        continue;
                    }
                    let (va, vb) = match ch.to_ascii_lowercase() {
                        'x' => (0xf, 0xf),
                        'z' => (0, 0xf),
                        c => (c.to_digit(if hex { 16 } else { 2 }).unwrap_or(0) as u64, 0),
                    };
                    for k in 0..bits {
                        let bit = pos + k;
                        if bit < r.width {
                            a[(bit / 64) as usize] |= ((va >> k) & 1) << (bit % 64);
                            b[(bit / 64) as usize] |= ((vb >> k) & 1) << (bit % 64);
                        }
                    }
                    pos += bits;
                }
                a.extend_from_slice(&b);
                self.apply(sig, Res::Elem(e as u32), &a, r.width);
            }
            addr += 1;
        }
    }

    pub fn finished(&self) -> bool {
        self.finished
    }
}

/// $random: the algorithm of IEEE 1364-2005 (rtl_dist_uniform-free variant used by $random).
fn random_step(seed: u32) -> (u32, u32) {
    let ns = seed.wrapping_mul(69069).wrapping_add(1);
    (ns, ns)
}

/// Evaluate straight-line/branching code without a scheduler (constant expressions and
/// constant functions at elaboration time). Stores update memory without notifications.
pub fn eval_const(code: &[Instr], mem: &mut [u64], sigs: &[SignalInfo]) -> Result<(), String> {
    let index = |m: &[u64], ix: &Index| -> Option<i64> {
        let v = ops::read_index(m, ix.slot as usize, ix.w, ix.signed)?;
        Some(if ix.neg { ix.k - v } else { v + ix.k })
    };
    let mut pc = 0usize;
    let mut steps = 0u64;
    while pc < code.len() {
        steps += 1;
        if steps > 50_000_000 {
            return Err("constant evaluation does not terminate".into());
        }
        let ins = &code[pc];
        pc += 1;
        let m = &mut *mem;
        match ins {
            Instr::Copy { d, s, w } => ops::copy(m, *d as usize, *s as usize, *w),
            Instr::Resize { d, s, sw, dw, sign } => ops::resize(m, *d as usize, *s as usize, *sw, *dw, *sign),
            Instr::Un { op, d, s, w } => ops::unop(m, *op, *d as usize, *s as usize, *w),
            Instr::Red { op, d, s, w } => ops::redop(m, *op, *d as usize, *s as usize, *w),
            Instr::Bin { op, d, a, b, w } => ops::binop(m, *op, *d as usize, *a as usize, *b as usize, *w),
            Instr::Cmp { op, d, a, b, w } => ops::cmpop(m, *op, *d as usize, *a as usize, *b as usize, *w),
            Instr::Shift { op, d, a, w, b, bw } => ops::shift(m, *op, *d as usize, *a as usize, *w, *b as usize, *bw),
            Instr::Pow { d, a, b, w, bw, signed } => ops::pow(m, *d as usize, *a as usize, *b as usize, *w, *bw, *signed),
            Instr::Mux { d, c, cw, t, f, w } => ops::mux(m, *d as usize, *c as usize, *cw, *t as usize, *f as usize, *w),
            Instr::Concat { d, w, parts } => ops::concat(m, *d as usize, *w, parts),
            Instr::Insert { d, dw, s, sw, off } => ops::insert(m, *d as usize, *dw, *s as usize, *sw, *off),
            Instr::Slice { d, s, sw, lsb, w } => ops::extract(m, *d as usize, *s as usize, *sw, *lsb as i64, *w),
            Instr::DynSlice { d, s, sw, ix, w } => match index(m, ix) {
                Some(off) => ops::extract(m, *d as usize, *s as usize, *sw, off, *w),
                None => ops::set_x(m, *d as usize, *w),
            },
            Instr::ArrRead { d, sig, ix } => {
                let r = &sigs[*sig as usize];
                match index(m, ix) {
                    Some(e) if e >= 0 && e < r.depth as i64 => {
                        let n = 2 * nw(r.width);
                        let src = r.slot as usize + e as usize * n;
                        m.copy_within(src..src + n, *d as usize);
                    }
                    _ => ops::set_x(m, *d as usize, r.width),
                }
            }
            Instr::Clog2 { d, s, w } => ops::clog2(m, *d as usize, *s as usize, *w),
            Instr::Jmp { to } => pc = *to as usize,
            Instr::Jz { c, w, to } => {
                if ops::truth(m, *c as usize, *w) != Some(true) {
                    pc = *to as usize;
                }
            }
            Instr::Halt => break,
            Instr::Store { t, s, mode: StoreMode::Blocking } => {
                let s = *s as usize;
                match *t {
                    Target::Whole { sig } => {
                        let r = &sigs[sig as usize];
                        ops::copy(m, r.slot as usize, s, r.width);
                    }
                    Target::Part { sig, lsb, w } => {
                        let r = &sigs[sig as usize];
                        let (n, ns) = (nw(r.width), nw(w));
                        for p in 0..2 {
                            let mut i = 0;
                            while i < w {
                                let len = (w - i).min(64);
                                let v = ops::get_bits(m, s + p * ns, i, len);
                                ops::set_bits(m, r.slot as usize + p * n, lsb + i, len, v);
                                i += len;
                            }
                        }
                    }
                    Target::DynPart { sig, ix, w } => {
                        let r = &sigs[sig as usize];
                        if let Some(off) = index(m, &ix) {
                            let (n, ns) = (nw(r.width), nw(w));
                            for i in 0..w as i64 {
                                let dst = off + i;
                                if dst < 0 || dst >= r.width as i64 {
                                    continue;
                                }
                                for p in 0..2 {
                                    let v = ops::get_bits(m, s + p * ns, i as u32, 1);
                                    ops::set_bits(m, r.slot as usize + p * n, dst as u32, 1, v);
                                }
                            }
                        }
                    }
                    Target::ElemK { sig, e } => {
                        let r = &sigs[sig as usize];
                        let n = 2 * nw(r.width);
                        ops::copy(m, r.slot as usize + e as usize * n, s, r.width);
                    }
                    Target::Elem { sig, ix } => {
                        let r = &sigs[sig as usize];
                        if let Some(e) = index(m, &ix) {
                            if e >= 0 && e < r.depth as i64 {
                                let n = 2 * nw(r.width);
                                ops::copy(m, r.slot as usize + e as usize * n, s, r.width);
                            }
                        }
                    }
                    _ => return Err("unsupported assignment in constant function".into()),
                }
            }
            _ => return Err("not a constant expression".into()),
        }
    }
    Ok(())
}

/// Ranks of the processes that can be evaluated in the combinational settle queue: level-sensitive
/// processes whose only suspension is one `Wait` (assigns, always @*, always_comb, ...). A process
/// gets a rank above every ranked process that writes a signal it reads. Processes in loops (and
/// everything else) get NONE and are scheduled through the normal active queue.
fn comb_ranks(prog: &Program) -> Vec<u32> {
    comb_graph(prog).0
}

/// (ranks, combinational successors of each eligible process, eligibility)
pub(crate) fn comb_graph(prog: &Program) -> (Vec<u32>, Vec<Vec<u32>>, Vec<bool>) {
    let n = prog.procs.len();
    let mut eligible = vec![false; n];
    // reads and writes are (signal, array element or NONE = whole signal); an alias of an array
    // element counts as that element
    let norm = |s: u32, e: u32| -> (u32, u32) {
        match prog.signals[s as usize].alias_of {
            Some(ae) if e == NONE => ae,
            _ => (s, e),
        }
    };
    let mut reads: Vec<Vec<(u32, u32)>> = vec![Vec::new(); n];
    for c in &prog.ctls {
        if c.proc == NONE {
            continue;
        }
        reads[c.proc as usize].extend(c.triggers.iter().map(|t| norm(t.0, t.2)));
    }
    let mut level = vec![true; n];
    for c in &prog.ctls {
        if c.proc != NONE && c.triggers.iter().any(|t| t.1 != Edge::Any) {
            level[c.proc as usize] = false;
        }
    }
    let mut writes: Vec<Vec<(u32, u32)>> = vec![Vec::new(); n];
    for (i, p) in prog.procs.iter().enumerate() {
        if p.class != ProcClass::Normal || !level[i] {
            continue;
        }
        let mut waits = 0;
        let mut ok = true;
        for ins in &p.code {
            match ins {
                Instr::Wait { .. } => waits += 1,
                Instr::Delay { .. } | Instr::Halt => ok = false,
                Instr::Sys(t) if matches!(**t, SysTask::Finish | SysTask::Monitor { .. } | SysTask::Strobe { .. }) => {
                    ok = false
                }
                Instr::Store { t, mode, .. } => {
                    if matches!(mode, StoreMode::NbaDelay(_)) {
                        ok = false;
                    }
                    let w = match t {
                        Target::ElemK { sig, e } => (*sig, *e),
                        Target::Whole { sig }
                        | Target::Part { sig, .. }
                        | Target::DynPart { sig, .. }
                        | Target::Elem { sig, .. }
                        | Target::ElemPart { sig, .. } => (*sig, NONE),
                    };
                    writes[i].push(norm(w.0, w.1));
                }
                _ => {}
            }
        }
        eligible[i] = ok && waits == 1;
    }
    // signal -> eligible readers
    let mut readers: Vec<Vec<(u32, u32)>> = vec![Vec::new(); prog.signals.len()];
    for i in 0..n {
        if eligible[i] {
            for &(s, e) in &reads[i] {
                readers[s as usize].push((i as u32, e));
            }
        }
    }
    // edges between eligible processes (Kahn's algorithm, longest path)
    let mut succ: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut indeg = vec![0u32; n];
    for i in 0..n {
        if !eligible[i] {
            continue;
        }
        let mut targets: Vec<u32> = Vec::new();
        for &(s, we) in &writes[i] {
            for &(r, re) in &readers[s as usize] {
                let overlap = we == NONE || re == NONE || we == re;
                if overlap && r as usize != i {
                    targets.push(r);
                }
            }
        }
        targets.sort_unstable();
        targets.dedup();
        for &t in &targets {
            indeg[t as usize] += 1;
        }
        succ[i] = targets;
    }
    let mut rank = vec![NONE; n];
    let mut ready: Vec<u32> = (0..n as u32).filter(|&i| eligible[i as usize] && indeg[i as usize] == 0).collect();
    for &i in &ready {
        rank[i as usize] = 0;
    }
    while let Some(i) = ready.pop() {
        let next = rank[i as usize] + 1;
        for &t in &succ[i as usize] {
            let t = t as usize;
            if rank[t] == NONE || rank[t] < next {
                rank[t] = next;
            }
            indeg[t] -= 1;
            if indeg[t] == 0 {
                ready.push(t as u32);
            }
        }
    }
    // processes left with indegree > 0 are in (or behind) a loop: schedule them normally
    for i in 0..n {
        if eligible[i] && indeg[i] > 0 {
            rank[i] = NONE;
        }
    }
    (rank, succ, eligible)
}

// ---------------------------------------------------------------------------------------------
// callbacks from JIT-compiled code

#[cfg(feature = "jit")]
unsafe extern "C" fn cb_notify(sim: *mut Sim, sig: u32, old: u32) {
    (*sim).notify(sig, old as u8);
}

#[cfg(feature = "jit")]
unsafe extern "C" fn cb_nba(sim: *mut Sim, sig: u32, slot: u32, a: u64, b: u64) {
    (*sim).push_small(sig, slot, a, b);
}

#[cfg(feature = "jit")]
unsafe extern "C" fn cb_nba_wide(sim: *mut Sim, sig: u32, s: u32, w: u32) {
    let sim = &mut *sim;
    let s = s as usize;
    let n = 2 * nw(w);
    let off = sim.nba_vals.len() as u32;
    sim.nba_vals.extend_from_slice(&sim.mem[s..s + n]);
    sim.push_generic(Nba { sig, res: Res::Whole, off, sw: w });
}

#[cfg(feature = "jit")]
unsafe extern "C" fn cb_gen(sim: *mut Sim, p: u32, pc: u32) -> u64 {
    let sim = &mut *sim;
    let code: &Code = &*Rc::as_ptr(&sim.code);
    let ins = &code.orig[p as usize][pc as usize];
    let mut npc = pc as usize + 1;
    if sim.exec_gen(p, ins, &mut npc) {
        u64::MAX
    } else {
        npc as u64
    }
}

#[cfg(feature = "jit")]
unsafe extern "C" fn cb_wait(sim: *mut Sim, p: u32, ctl: u32, pc: u32) {
    let sim = &mut *sim;
    let pr = &mut sim.procs[p as usize];
    pr.wait = ctl;
    pr.pc = pc;
}

#[cfg(feature = "jit")]
unsafe extern "C" fn cb_delay(sim: *mut Sim, p: u32, s: u32, pc: u32) {
    let sim = &mut *sim;
    let s = s as usize;
    let d = if sim.mem[s + 1] == 0 { sim.mem[s] } else { 0 };
    if d == 0 {
        sim.inactive.push(p);
    } else {
        sim.future.entry(sim.time + d).or_default().push(Fut::Wake(p));
    }
    sim.procs[p as usize].pc = pc;
}

#[cfg(feature = "jit")]
unsafe extern "C" fn cb_halt(sim: *mut Sim, p: u32, pc: u32) {
    let sim = &mut *sim;
    let pr = &mut sim.procs[p as usize];
    pr.pc = pc;
    pr.wait = NONE;
}
