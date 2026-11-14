//! The event scheduler (IEEE 1364 stratified regions) and the bytecode interpreter.

use crate::fast::{self, Op};
use crate::ops;
use crate::history::History;
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

/// Process state. `pc`, `wait` and `wseq` are written directly by JIT code (offsets 0, 4, 8).
#[derive(Clone, Copy)]
#[repr(C)]
struct ProcRt {
    pc: u32,
    wait: u32,
    /// when the process started waiting (Hot::wseq): waiters woken together run newest first,
    /// like Icarus (whose waiting lists are LIFO)
    wseq: u64,
    class: ProcClass,
    started: bool,
    /// position in rank order in the combinational settle set, NONE if scheduled normally
    rank: u32,
    /// incremented when the process is disabled or restarted: older wake-ups are ignored
    gen: u32,
    /// fork branches still to finish before this process continues (join)
    joining: u32,
    /// the forking process of a branch, NONE otherwise
    parent: u32,
    /// a fork branch that has finished (or was disabled)
    done: bool,
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
    /// counter for ProcRt::wseq (offset 32)
    pub wseq: u64,
}

enum Fut {
    /// (process, generation)
    Wake(u32, u32),
    Nba { sig: u32, res: Res, sw: u32, val: Box<[u64]> },
    /// a delayed continuous-assignment update, valid while its generation is current
    Inertial { key: (u32, u32), gen: u32, sig: u32, res: Res, sw: u32, val: Box<[u64]> },
}

/// A subroutine call of a recursive function: the memory it saved, where to return to and
/// where its results go.
struct Frame {
    saved: Vec<(u32, Vec<u64>)>,
    ret: u32,
    dsts: Box<[u32]>,
}

/// Why `advance` returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    /// the requested time was reached; more events are scheduled
    Paused,
    /// nothing is scheduled any more
    Idle,
    /// $finish
    Finished,
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
    /// edge groups: members (process ids); group g is pseudo process nprocs + g
    groups: Vec<Vec<u32>>,
    /// number of real processes
    nprocs: u32,
    /// signal -> processes that store to it (for Kick)
    writers: HashMap<u32, Vec<u32>>,
    /// forced bits per signal (value-plane words); stores keep these bits
    force_mask: HashMap<u32, Vec<u64>>,
    /// recursion: per process, the saved frames of active subroutine calls
    frames: HashMap<u32, Vec<Frame>>,
    /// pending inertial (continuous-assignment delay) updates: target -> current generation
    inertial: HashMap<(u32, u32), u32>,
    /// a $dumpvars block is due at the end of the current time step
    dump_pending: bool,
    /// a force store is writing (the mask does not apply)
    force_bypass: bool,
    /// per group: run the members last to first next time
    group_rev: Vec<bool>,
    /// (array, element) -> virtual signal of that element
    elem_sig: HashMap<(u32, u32), u32>,
    /// per signal: some element of this array has its own (virtual or alias) signal
    has_elem: Vec<bool>,
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
    /// active processes with their wake sequence number and process generation
    active: VecDeque<(u32, u64, u32)>,
    /// wake sequence counter (FIFO order between active wakes and the combinational settle)
    seq: u64,
    /// sequence number of the first pending combinational wake, u64::MAX if none is pending
    comb_seq: u64,
    inactive: Vec<(u32, u32)>,
    nba: Vec<Nba>,
    nba_vals: Vec<u64>,
    /// the NBA queue (length in `hot.nba_len`); `nba` holds the generic entries it refers to
    nbaq: Vec<NbaE>,
    hot: Hot,
    /// processes woken by one change, before they are queued (newest waiter first)
    wake_buf: Vec<(u64, u32)>,
    future: BTreeMap<u64, Vec<Fut>>,
    postponed: Vec<u32>,
    pub time: u64,
    finished: bool,
    out: BufWriter<Box<dyn Write>>,
    monitor: u32,
    monitor_on: bool,
    seed: i32,
    /// shared seed of $urandom / $urandom_range
    useed: i32,
    vcd_path: String,
    vcd: Option<Vcd>,
    dumping: bool,
    /// signal changes are collected in `dirty` (dumping or recording history)
    track: bool,
    /// value history for the debug server
    history: Option<History>,
    /// all values are recorded at the end of the current time step
    history_pending: bool,
    dirty: Vec<u32>,
    is_dirty: Vec<bool>,
    /// open files
    files: HashMap<u32, FileH>,
    next_fd: u32,
    scratch: Vec<u64>,
    plusargs: Vec<String>,
    /// FX_GENSTATS: count generic-path instructions by kind, printed at the end (debugging)
    genstats: Option<HashMap<String, u64>>,
    /// FX_TRACE=<time>: print every process activation from that time on to stderr (debugging)
    trace: u64,
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
        if std::env::var_os("FX_GENSTATS").is_some() {
            let mut per: HashMap<u32, (Vec<u32>, Vec<u32>, [u32; 3])> = HashMap::new();
            for (ci, c) in prog.ctls.iter().enumerate() {
                for &(sig, edge, _) in &c.triggers {
                    let e = per.entry(sig).or_default();
                    e.0.push(c.proc);
                    e.1.push(ci as u32);
                    e.2[edge as usize] += 1;
                }
            }
            for (sig, (mut ps, cs, edges)) in per {
                if cs.len() > 50 {
                    ps.sort_unstable();
                    ps.dedup();
                    let mut names: HashMap<&str, u32> = HashMap::new();
                    for &p in &ps {
                        *names.entry(prog.procs[p as usize].name.as_str()).or_default() += 1;
                    }
                    let mut nv: Vec<_> = names.into_iter().collect();
                    nv.sort_by_key(|x| std::cmp::Reverse(x.1));
                    eprintln!("   {:?}", &nv[..nv.len().min(8)]);
                    eprintln!("fan {} ctls {} procs {} any/pos/neg {:?}", prog.signals[sig as usize].name, cs.len(), ps.len(), edges);
                }
            }
        }
        let ntot = nsig + vsig_of.len();
        // edge groups: clocked processes with the same triggers run as one activation
        let (groups, group_of) = edge_groups(&prog, &ranks);
        let nprocs = prog.procs.len() as u32;
        let ngroups = groups.len() as u32;
        // fan-out tables: combinational readers by comb index, everything else as events
        let mut per_sig: Vec<Vec<Fan>> = vec![Vec::new(); ntot];
        let mut per_sig_c: Vec<Vec<u32>> = vec![Vec::new(); ntot];
        for (ci, c) in prog.ctls.iter().enumerate() {
            // a group member's wait is represented by its group's (one entry, at the first member)
            let (proc, ctl) = match c.proc {
                NONE => (NONE, ci as u32),
                p => match group_of[p as usize] {
                    NONE => (p, ci as u32),
                    g if groups[g as usize].0[0] == p => (nprocs + g, NONE - 1 - g),
                    _ => continue,
                },
            };
            for &(sig, edge, e) in &c.triggers {
                let sig = if e != NONE { elem_sig[&(sig, e)] } else { sig };
                if proc != NONE && proc < nprocs && ranks[proc as usize] != NONE {
                    per_sig_c[sig as usize].push(ranks[proc as usize]);
                } else {
                    per_sig[sig as usize].push(Fan { proc, ctl, edge });
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
        let mut procs: Vec<ProcRt> = prog
            .procs
            .iter()
            .enumerate()
            .map(|(i, p)| ProcRt {
                pc: 0,
                wait: NONE,
                wseq: 0,
                class: p.class,
                started: false,
                rank: ranks[i],
                gen: 0,
                joining: 0,
                parent: p.parent,
                done: p.class == ProcClass::Spawned,
            })
            .collect();
        // one pseudo process per edge group, waiting on its group control from the start
        for g in 0..ngroups {
            procs.push(ProcRt {
                pc: 0,
                wait: NONE - 1 - g,
                wseq: 0,
                class: ProcClass::Normal,
                started: true,
                rank: NONE,
                gen: 0,
                joining: 0,
                parent: NONE,
                done: false,
            });
        }
        let groups: Vec<Vec<u32>> = groups.into_iter().map(|g| g.0).collect();
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
            group_rev: vec![true; groups.len()],
            force_mask: HashMap::new(),
            inertial: HashMap::new(),
            frames: HashMap::new(),
            dump_pending: false,
            force_bypass: false,
            writers: {
                let mut m: HashMap<u32, Vec<u32>> = HashMap::new();
                for (p, pd) in prog.procs.iter().enumerate() {
                    for ins in &pd.code {
                        if let Instr::Store { t, .. } = ins {
                            let sig = match *t {
                                Target::Whole { sig }
                                | Target::Part { sig, .. }
                                | Target::DynPart { sig, .. }
                                | Target::Elem { sig, .. }
                                | Target::ElemK { sig, .. }
                                | Target::ElemPart { sig, .. }
                    | Target::ElemDynPart { sig, .. } => sig,
                            };
                            let v = m.entry(sig).or_default();
                            if !v.contains(&(p as u32)) {
                                v.push(p as u32);
                            }
                        }
                    }
                }
                m
            },
            groups,
            nprocs,
            cur_comb: NONE,
            has_elem: {
                let mut v = vec![false; nsig];
                for &(a, _) in elem_sig.keys() {
                    v[a as usize] = true;
                }
                v
            },
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
            hot: Hot { nba_ptr: std::ptr::null_mut(), nba_len: 0, nba_cap: 0, procs: std::ptr::null_mut(), wseq: 0 },
            wake_buf: Vec::new(),
            future: BTreeMap::new(),
            postponed: Vec::new(),
            time: 0,
            finished: false,
            out: BufWriter::with_capacity(1 << 16, Box::new(std::io::stdout())),
            monitor: NONE,
            monitor_on: true,
            seed: 0,
            useed: 0,
            vcd_path: "dump.vcd".into(),
            vcd: None,
            dumping: false,
            track: false,
            history: None,
            history_pending: false,
            dirty: Vec::new(),
            is_dirty: vec![false; nsig],
            files: HashMap::new(),
            next_fd: 3,
            scratch: Vec::new(),
            plusargs: Vec::new(),
            genstats: std::env::var_os("FX_GENSTATS").map(|_| HashMap::new()),
            trace: std::env::var("FX_TRACE").ok().and_then(|v| v.parse().ok()).unwrap_or(u64::MAX),
            stats: Stats::default(),
            #[cfg(feature = "jit")]
            jit: None,
            #[cfg(feature = "jit")]
            entries: Vec::new(),
            prog,
        }
    }

    /// Run until no events remain, $finish, or `max_time` is passed.
    /// Run until no events remain, $finish, or `max_time` is passed.
    pub fn run(&mut self, max_time: Option<u64>) {
        self.start();
        self.advance(max_time);
        self.finish();
    }

    /// Start every process (time 0); then `advance` runs the simulation.
    pub fn start(&mut self) {
        fmt::set_precision(self.prog.precision);
        fmt::set_time_format(None);
        // combinational processes first: they evaluate (assigns) or reach their first wait
        // (always @*) before any other process can change their inputs at time 0
        for i in 0..self.nprocs as usize {
            let p = &mut self.procs[i];
            if p.class == ProcClass::Normal && p.rank != NONE {
                p.started = true;
                let c = p.rank as usize;
                self.comb_bits[c >> 6] |= 1 << (c & 63);
                self.comb_lo = self.comb_lo.min(c >> 6);
                self.comb_wake();
            }
        }
        for i in 0..self.nprocs as usize {
            let p = &mut self.procs[i];
            if p.class == ProcClass::Normal && p.rank == NONE {
                p.started = true;
                self.push_active(i as u32);
            }
        }
    }

    /// Run the current time step and the following ones up to and including `until`.
    pub fn advance(&mut self, until: Option<u64>) -> RunState {
        self.advance_steps(until, u64::MAX)
    }

    /// `advance`, but stop (Paused) after `max_steps` time steps; the time is then that of the
    /// last step run.
    pub fn advance_steps(&mut self, until: Option<u64>, max_steps: u64) -> RunState {
        let mut steps = 0;
        loop {
            self.stats.time_steps += 1;
            self.timestep();
            if self.finished {
                return RunState::Finished;
            }
            let Some(&t) = self.future.keys().next() else { return RunState::Idle };
            steps += 1;
            if steps >= max_steps {
                return RunState::Paused;
            }
            if let Some(mt) = until {
                if t > mt {
                    self.time = mt.max(self.time);
                    return RunState::Paused;
                }
            }
            let (t, evs) = self.future.pop_first().unwrap();
            self.time = t;
            for ev in evs {
                match ev {
                    Fut::Wake(p, g) => {
                        if self.procs[p as usize].gen == g {
                            self.push_active(p);
                        }
                    }
                    Fut::Nba { sig, res, sw, val } => {
                        let off = self.nba_vals.len() as u32;
                        self.nba_vals.extend_from_slice(&val);
                        self.push_generic(Nba { sig, res, off, sw });
                    }
                    Fut::Inertial { key, gen, sig, res, sw, val } => {
                        if self.inertial.get(&key) == Some(&gen) {
                            self.inertial.remove(&key);
                            self.apply(sig, res, &val, sw);
                        }
                    }
                }
            }
        }
    }

    /// The time of the next scheduled event, if any.
    pub fn next_event_time(&self) -> Option<u64> {
        self.future.keys().next().copied()
    }

    /// Run the final blocks and flush the outputs.
    pub fn finish(&mut self) {
        if let (Some(v), true) = (&mut self.vcd, self.dumping) {
            v.end(self.time);
        }
        let finals: Vec<u32> =
            (0..self.procs.len() as u32).filter(|&p| self.procs[p as usize].class == ProcClass::Final).collect();
        self.finished = false;
        for p in finals {
            self.exec(p);
        }
        self.flush();
    }

    // ---------------------------------------------------------------- interactive access

    /// Send the simulation's output ($display & co.) to `w` instead of stdout.
    pub fn set_output(&mut self, w: Box<dyn Write>) {
        let _ = self.out.flush();
        self.out = BufWriter::with_capacity(1 << 16, w);
    }

    /// The simulation time.
    pub fn now(&self) -> u64 {
        self.time
    }

    /// Whether $finish has been called.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Signal names (hierarchical) and ids.
    pub fn signal_names(&self) -> &[(String, u32)] {
        &self.prog.names
    }

    /// Width and array depth (0: not an array) of a signal.
    pub fn signal_shape(&self, sig: u32) -> (u32, u32) {
        let r = self.sigs[sig as usize];
        (r.width, r.depth)
    }

    /// The value of a (non-array) signal formatted in `radix` ('b', 'h', 'o', 'd').
    pub fn format_signal(&self, sig: u32, radix: char) -> String {
        let r = self.sigs[sig as usize];
        let n = nw(r.width);
        let s = r.slot as usize;
        let info = &self.prog.signals[sig as usize];
        if info.real {
            return fmt::fmt_real(f64::from_bits(self.mem[s]), 'g', 6);
        }
        let v = fmt::Val { a: &self.mem[s..s + n], b: &self.mem[s + n..s + 2 * n], w: r.width, signed: info.signed };
        match radix {
            'b' => fmt::fmt_radix(&v, 1, true),
            'o' => fmt::fmt_radix(&v, 3, true),
            'd' => fmt::fmt_dec(&v, false),
            _ => fmt::fmt_radix(&v, 4, true),
        }
    }

    /// Start a VCD of the signals whose name `keep` accepts (replacing an open one); the current
    /// values are written at the current time.
    pub fn trace_start(&mut self, path: &str, keep: &dyn Fn(&str) -> bool) -> std::io::Result<()> {
        if let Some(v) = &mut self.vcd {
            v.flush();
        }
        let v = Vcd::create_filtered(path, &self.prog, keep).inspect_err(|e| {
            eprintln!("warning: cannot create VCD file {path}: {e}");
        })?;
        // the initial values are written at the end of the current time step
        self.vcd = Some(v);
        self.dumping = true;
        self.track = true;
        self.dump_pending = true;
        Ok(())
    }

    /// Pause (false) or resume (true) dumping; resuming writes all traced values.
    pub fn trace_enable(&mut self, on: bool) {
        if self.vcd.is_none() || self.dumping == on {
            return;
        }
        if on {
            let v = self.vcd.as_mut().unwrap();
            v.time(self.time);
            v.reset_values();
            for (i, r) in self.sigs.iter().enumerate() {
                if r.depth == 0 {
                    v.write_value(i as u32, &self.mem, r.slot, r.width);
                }
            }
        }
        self.dumping = on;
        self.track = on || self.history.is_some();
    }

    /// The number of traced signals (0: no trace).
    pub fn traced_signals(&self) -> usize {
        self.vcd.as_ref().map_or(0, |v| v.ids.iter().filter(|c| !c.is_empty()).count())
    }

    /// Whether a (non-array) signal has exactly this value (value and unknown planes).
    pub fn signal_equals(&self, sig: u32, planes: &[u64]) -> bool {
        let r = self.sigs[sig as usize];
        let n = 2 * nw(r.width);
        self.mem[r.slot as usize..r.slot as usize + n] == planes[..n]
    }

    /// Set a signal (value and unknown planes, `width` bits) as a blocking assignment would;
    /// processes it wakes run at the next `advance`.
    pub fn deposit(&mut self, sig: u32, planes: &[u64]) {
        let w = self.sigs[sig as usize].width;
        self.apply(sig, Res::Whole, planes, w);
    }

    /// Force a signal to a value until `release`.
    pub fn force(&mut self, sig: u32, planes: &[u64]) {
        let w = self.sigs[sig as usize].width;
        // the value goes through a scratch slot (no process is running)
        let n = 2 * nw(w);
        self.mem[..n].copy_from_slice(&planes[..n]);
        self.store(&Target::Whole { sig }, 0, StoreMode::Force);
    }

    /// Release a forced signal: a net takes its drivers' value again, a variable keeps it.
    pub fn release(&mut self, sig: u32) {
        self.force_mask.remove(&sig);
        self.kick(sig);
    }

    /// Re-evaluate the waiting processes that write `sig`.
    fn kick(&mut self, sig: u32) {
        let ws = self.writers.get(&sig).cloned().unwrap_or_default();
        for w in ws {
            let pr = &mut self.procs[w as usize];
            if pr.wait != NONE && pr.class != ProcClass::Spawned {
                pr.wait = NONE;
                if pr.rank != NONE {
                    let c = pr.rank as usize;
                    self.comb_bits[c >> 6] |= 1 << (c & 63);
                    self.comb_lo = self.comb_lo.min(c >> 6);
                    self.comb_wake();
                } else {
                    self.push_active(w);
                }
            }
        }
    }

    pub fn flush(&mut self) {
        if let Some(st) = &self.genstats {
            let mut v: Vec<_> = st.iter().collect();
            v.sort_by_key(|x| std::cmp::Reverse(*x.1));
            for (k, n) in v.iter().take(25) {
                eprintln!("gen {n:>12} {k}");
            }
        }
        let _ = self.out.flush();
        if let Some(v) = &mut self.vcd {
            v.flush();
        }
        for f in self.files.values_mut() {
            if let FileH::W(f) = f {
                let _ = f.flush();
            }
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
            if let Some((p, _, g)) = self.active.pop_front() {
                if self.procs[p as usize].gen != g {
                    continue; // disabled or restarted since it was woken
                }
                self.exec(p);
                if self.finished {
                    return;
                }
                continue;
            }
            if !self.inactive.is_empty() {
                for (p, g) in std::mem::take(&mut self.inactive) {
                    if self.procs[p as usize].gen == g {
                        self.push_active(p);
                    }
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
        if self.history.is_some() {
            self.record_history();
        }
        if self.dump_pending {
            // $dumpvars / a trace started in this time step: the values at its end
            self.dump_pending = false;
            let v = self.vcd.as_mut().unwrap();
            v.time(self.time);
            v.begin_dumpvars();
            for (i, r) in self.sigs.iter().enumerate() {
                if r.depth == 0 {
                    v.write_value(i as u32, &self.mem, r.slot, r.width);
                }
            }
            v.end_dumpvars();
            for &s in &self.dirty {
                self.is_dirty[s as usize] = false;
            }
            self.dirty.clear();
        } else if self.dumping && !self.dirty.is_empty() {
            self.dump_dirty();
        } else if !self.dirty.is_empty() {
            for &s in &self.dirty {
                self.is_dirty[s as usize] = false;
            }
            self.dirty.clear();
        }
    }

    /// Append this time step's changes (all values after `history_start`) to the history.
    fn record_history(&mut self) {
        let h = self.history.as_mut().unwrap();
        let t = self.time;
        let mut rec = |s: u32, r: &SigRt| {
            let n = 2 * nw(r.width);
            h.record(s, t, &self.mem[r.slot as usize..r.slot as usize + n]);
        };
        if self.history_pending {
            self.history_pending = false;
            for s in 0..self.nreal {
                rec(s, &self.sigs[s as usize]);
            }
        } else {
            for &s in &self.dirty {
                rec(s, &self.sigs[s as usize]);
            }
        }
    }

    /// Record the value history of every non-array signal from the end of the current time
    /// step on, using at most `budget` bytes.
    pub fn history_start(&mut self, budget: usize) {
        let widths: Vec<u32> = (0..self.prog.signals.len())
            .map(|s| {
                let r = &self.sigs[s];
                if r.depth == 0 && (s as u32) < self.nreal { r.width } else { 0 }
            })
            .collect();
        self.history = Some(History::new(&widths, budget));
        self.history_pending = true;
        self.track = true;
    }

    /// The recorded value history.
    pub fn history(&self) -> Option<&History> {
        self.history.as_ref()
    }

    /// The recorded value history, to index it (`History::prepare`).
    pub fn history_mut(&mut self) -> Option<&mut History> {
        self.history.as_mut()
    }

    /// Declaration info of a signal.
    pub fn signal_info(&self, sig: u32) -> &crate::SignalInfo {
        &self.prog.signals[sig as usize]
    }

    /// The time precision (power of ten of a tick in seconds).
    pub fn precision(&self) -> i32 {
        self.prog.precision
    }

    #[inline]
    fn push_active(&mut self, p: u32) {
        let g = self.procs[p as usize].gen;
        self.active.push_back((p, self.seq, g));
        self.seq += 1;
    }

    /// Suspend `p` for `d` ticks (0: the inactive region of this time step).
    #[inline]
    fn delay(&mut self, p: u32, d: u64) {
        let g = self.procs[p as usize].gen;
        if d == 0 {
            self.inactive.push((p, g));
        } else {
            self.future.entry(self.time + d).or_default().push(Fut::Wake(p, g));
        }
    }

    /// A process ran to its end: a finished fork branch lets its parent's join count down.
    fn proc_done(&mut self, p: u32) {
        let pr = &mut self.procs[p as usize];
        if pr.class != ProcClass::Spawned || pr.done {
            return;
        }
        pr.done = true;
        let parent = pr.parent;
        if parent != NONE && self.procs[parent as usize].joining > 0 {
            let pp = &mut self.procs[parent as usize];
            pp.joining -= 1;
            if pp.joining == 0 {
                self.push_active(parent);
            }
        }
    }

    /// Start fork branches. Returns true if the parent waits for them.
    fn fork(&mut self, p: u32, children: &[u32], join: u8) -> bool {
        // IEEE leaves the order open; Icarus starts the branches last to first
        for &c in children.iter().rev() {
            let ch = &mut self.procs[c as usize];
            if !ch.done {
                eprintln!("warning: fork branch '{}' restarted while still running", self.prog.procs[c as usize].name);
            }
            ch.pc = 0;
            ch.wait = NONE;
            ch.gen = ch.gen.wrapping_add(1);
            ch.done = false;
            ch.joining = 0;
            self.push_active(c);
        }
        let n = children.len() as u32;
        let joining = match join {
            0 => n,
            1 => n.min(1),
            _ => 0,
        };
        self.procs[p as usize].joining = joining;
        joining > 0
    }

    /// End the named blocks of a disable target: their fork branches stop, a process inside a
    /// block continues after it. Returns (new pc of `p` if it continues, whether `p` stopped).
    fn disable(&mut self, p: u32, target: u32, pc: usize) -> (Option<usize>, bool) {
        let mut new_pc = None;
        let mut stopped = false;
        if target == NONE {
            // disable fork: every branch started by `p`, and their branches
            let mut stack = vec![p];
            while let Some(q) = stack.pop() {
                for c in 0..self.nprocs {
                    let ch = &mut self.procs[c as usize];
                    if ch.parent == q && ch.class == ProcClass::Spawned && !ch.done {
                        ch.done = true;
                        ch.wait = NONE;
                        ch.gen = ch.gen.wrapping_add(1);
                        ch.joining = 0;
                        stack.push(c);
                    }
                }
            }
            return (None, false);
        }
        for &b in &self.prog.disables[target as usize].clone() {
            let blk = self.prog.blocks[b as usize].clone();
            for &c in &blk.children {
                let ch = &mut self.procs[c as usize];
                if !ch.done {
                    ch.done = true;
                    ch.wait = NONE;
                    ch.gen = ch.gen.wrapping_add(1);
                    ch.joining = 0;
                    if c == p {
                        stopped = true;
                    }
                }
            }
            let q = blk.proc;
            if q == p {
                // the disabling process itself: continue after the block if it is inside it
                if !stopped && pc > blk.start as usize && pc <= blk.end as usize {
                    new_pc = Some(blk.end as usize);
                }
                continue;
            }
            let pr = &mut self.procs[q as usize];
            let inside = pr.pc > blk.start && pr.pc <= blk.end && !(pr.class == ProcClass::Spawned && pr.done);
            if inside {
                pr.pc = blk.end;
                pr.wait = NONE;
                pr.joining = 0;
                pr.gen = pr.gen.wrapping_add(1);
                self.push_active(q);
            }
        }
        (new_pc, stopped)
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
        if let Some(st) = &mut self.genstats {
            let name = if (sig as usize) < self.prog.signals.len() { self.prog.signals[sig as usize].name.clone() } else { format!("vsig{sig}") };
            *st.entry(format!("notify {name} fan={} cfan={}", r.fan_hi - r.fan_lo, r.cfan_hi - r.cfan_lo)).or_default() += 1;
        }
        if r.parent != NONE {
            self.notify(r.parent, 0);
        }
        if self.track && sig < self.nreal && !self.is_dirty[sig as usize] {
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
                self.wake_buf.push((pr.wseq, f.proc));
            }
        }
        match self.wake_buf.len() {
            0 => {}
            1 => {
                let p = self.wake_buf[0].1;
                self.wake_buf.clear();
                self.push_active(p);
            }
            _ => {
                // newest waiter first (Icarus order)
                let mut buf = std::mem::take(&mut self.wake_buf);
                buf.sort_unstable_by_key(|w| std::cmp::Reverse(w.0));
                for &(_, p) in &buf {
                    self.push_active(p);
                }
                buf.clear();
                self.wake_buf = buf;
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
        // forced bits keep their value (unless this is the force itself)
        let snap = if !self.force_bypass && r.depth == 0 && self.force_mask.contains_key(&sig) {
            Some(self.mem[base..base + 2 * n].to_vec())
        } else {
            None
        };
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
        let changed = match snap {
            Some(old_words) => {
                let m = &self.force_mask[&sig];
                for i in 0..n {
                    let k = m[i];
                    self.mem[base + i] = (self.mem[base + i] & !k) | (old_words[i] & k);
                    self.mem[base + n + i] = (self.mem[base + n + i] & !k) | (old_words[n + i] & k);
                }
                self.mem[base..base + 2 * n] != old_words[..]
            }
            None => changed,
        };
        if changed {
            let e = match res {
                Res::Elem(e) | Res::ElemBits { e, .. } => e,
                _ => NONE,
            };
            if e != NONE {
                self.notify_elem(sig, e);
            } else {
                self.notify(sig, old);
            }
        }
    }

    /// The bit range [lsb, lsb + w) a resolved store covers in a (non-array) signal.
    fn res_bits(&self, sig: u32, res: Res) -> Option<(u32, u32)> {
        match res {
            Res::Whole => Some((0, self.sigs[sig as usize].width)),
            Res::Bits { lsb, w, .. } => Some((lsb, w)),
            _ => None,
        }
    }

    /// An element of an array changed: notify its own signal if it has one, else the array.
    #[inline]
    fn notify_elem(&mut self, sig: u32, e: u32) {
        if self.has_elem[sig as usize] {
            if let Some(&v) = self.elem_sig.get(&(sig, e)) {
                self.notify(v, 0);
                return;
            }
        }
        self.notify(sig, 0);
    }

    /// Blocking store of bits [lsb, lsb+w) of a ≤64-bit array element.
    #[inline]
    fn store_elem_part(&mut self, sig: u32, ea: u64, eb: u64, lsb: u8, w: u8, a: u64, b: u64) {
        let r = self.sigs[sig as usize];
        if eb != 0 || ea >= r.depth as u64 {
            return;
        }
        let d = r.slot as usize + 2 * ea as usize;
        let mask = fast::mk(w) << lsb;
        let na = (self.mem[d] & !mask) | ((a << lsb) & mask);
        let nb = (self.mem[d + 1] & !mask) | ((b << lsb) & mask);
        if self.mem[d] != na || self.mem[d + 1] != nb {
            self.mem[d] = na;
            self.mem[d + 1] = nb;
            self.notify_elem(sig, ea as u32);
        }
    }

    /// Non-blocking store of a ≤64-bit array element (the index is taken now).
    #[inline]
    fn nba_elem(&mut self, sig: u32, ea: u64, eb: u64, a: u64, b: u64) {
        let r = self.sigs[sig as usize];
        if eb != 0 || ea >= r.depth as u64 {
            return;
        }
        let slot = r.slot + 2 * ea as u32;
        // notify the element's own signal if it has one, else the array
        let target = if self.has_elem[sig as usize] {
            self.elem_sig.get(&(sig, ea as u32)).copied().unwrap_or(sig)
        } else {
            sig
        };
        self.push_small(target, slot, a, b);
    }

    /// Blocking store of bits [lsb, lsb+w) (w ≤ 64) of a wide signal (`n` words per plane).
    #[inline]
    fn store_part_wide(&mut self, sig: u32, slot: u32, lsb: u32, w: u8, n: u8, a: u64, b: u64) {
        let (base, n) = (slot as usize, n as usize);
        let old = ((self.mem[base] & 1) | ((self.mem[base + n] & 1) << 1)) as u8;
        let mut changed = false;
        for (plane, v) in [(0usize, a), (1usize, b)] {
            let o = ops::get_bits(&self.mem, base + plane * n, lsb, w as u32);
            if o != v {
                ops::set_bits(&mut self.mem, base + plane * n, lsb, w as u32, v);
                changed = true;
            }
        }
        if changed {
            self.notify(sig, old);
        }
    }

    /// Blocking store of a ≤64-bit array element; the index (value ea, unknown eb) is a plain
    /// element number. Out-of-range or unknown indices write nothing.
    #[inline]
    fn store_elem(&mut self, sig: u32, ea: u64, eb: u64, a: u64, b: u64) {
        let r = self.sigs[sig as usize];
        if eb != 0 || ea >= r.depth as u64 {
            return;
        }
        let d = r.slot as usize + 2 * ea as usize;
        if self.mem[d] != a || self.mem[d + 1] != b {
            self.mem[d] = a;
            self.mem[d + 1] = b;
            self.notify_elem(sig, ea as u32);
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
            Target::ElemDynPart { sig, ix, bix, w } => {
                let e = self.index(&ix)?;
                let r = &self.sigs[sig as usize];
                if e < 0 || e >= r.depth as i64 {
                    return None;
                }
                let off = self.index(&bix)?;
                let lo = off.max(0);
                let hi = (off + w as i64).min(r.width as i64);
                if lo >= hi {
                    return None;
                }
                Some((sig, Res::ElemBits { e: e as u32, lsb: lo as u32, w: (hi - lo) as u32, soff: (lo - off) as u32 }, w))
            }
        }
    }

    fn store(&mut self, t: &Target, s: u32, mode: StoreMode) {
        let Some((sig, res, sw)) = self.resolve(t) else { return };
        let n = 2 * nw(sw);
        let s = s as usize;
        match mode {
            StoreMode::Force => {
                if let Some((lsb, w)) = self.res_bits(sig, res) {
                    let words = nw(self.sigs[sig as usize].width);
                    let m = self.force_mask.entry(sig).or_insert_with(|| vec![0; words]);
                    for b in lsb..lsb + w {
                        m[(b / 64) as usize] |= 1 << (b % 64);
                    }
                }
                let tmp: Vec<u64> = self.mem[s..s + n].to_vec();
                self.force_bypass = true;
                self.apply(sig, res, &tmp, sw);
                self.force_bypass = false;
            }
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
                let forceable = (sig as usize) < self.prog.signals.len() && self.prog.signals[sig as usize].forced;
                if n == 2 && matches!(res, Res::Whole) && !forceable {
                    let slot = self.sigs[sig as usize].slot;
                    let (a, b) = (self.mem[s], self.mem[s + 1]);
                    self.push_small(sig, slot, a, b);
                    return;
                }
                let off = self.nba_vals.len() as u32;
                self.nba_vals.extend_from_slice(&self.mem[s..s + n]);
                self.push_generic(Nba { sig, res, off, sw });
            }
            StoreMode::Inertial(ds) => {
                // the delay: per changed bit rise (to 1), fall (to 0), turn-off (to z) or the
                // smallest (to x), the largest of those; word 3 = 1: only the LSB's transition
                // decides (module paths; rise when the LSB does not change)
                let ds = ds as usize;
                let (r, f, t, lsb_rule) = (self.mem[ds], self.mem[ds + 1], self.mem[ds + 2], self.mem[ds + 3] == 1);
                let t = if t == u64::MAX { r.min(f) } else { t };
                let hn = n / 2;
                let rs = self.sigs[sig as usize];
                let nws = nw(rs.width);
                let (base, lsb0) = match res {
                    Res::Whole => (rs.slot as usize, 0),
                    Res::Bits { lsb, soff, .. } => (rs.slot as usize, lsb as i64 - soff as i64),
                    Res::Elem(e) => (rs.slot as usize + e as usize * 2 * nws, 0),
                    Res::ElemBits { e, lsb, soff, .. } => (rs.slot as usize + e as usize * 2 * nws, lsb as i64 - soff as i64),
                };
                let bit_delay = |na: u64, nb: u64| match (na, nb) {
                    (1, 0) => r,
                    (0, 0) => f,
                    (0, _) => t,
                    _ => r.min(f).min(t),
                };
                let mut d = 0u64;
                let mut changed = false;
                let nbits = if lsb_rule { 1 } else { sw };
                for i in 0..nbits {
                    let (na, nb) = (ops::get_bits(&self.mem, s, i, 1), ops::get_bits(&self.mem, s + hn, i, 1));
                    let tb = lsb0 + i as i64;
                    let (oa, ob) = if tb >= 0 && (tb as u32) < rs.width {
                        (ops::get_bits(&self.mem, base, tb as u32, 1), ops::get_bits(&self.mem, base + nws, tb as u32, 1))
                    } else {
                        (na, nb)
                    };
                    if (oa, ob) != (na, nb) {
                        changed = true;
                        d = d.max(bit_delay(na, nb));
                    }
                }
                if lsb_rule && !changed {
                    // the LSB stays: the rise delay if anything else changes
                    d = r;
                }
                let lsb = match res {
                    Res::Bits { lsb, .. } | Res::ElemBits { lsb, .. } => lsb,
                    Res::Elem(e) => e,
                    Res::Whole => 0,
                };
                let key = (sig, lsb);
                let gen = self.inertial.get(&key).map_or(1, |g| g.wrapping_add(1));
                let val: Box<[u64]> = self.mem[s..s + n].into();
                if d == 0 {
                    self.inertial.remove(&key);
                    self.apply(sig, res, &val, sw);
                } else {
                    self.inertial.insert(key, gen);
                    self.future.entry(self.time + d).or_default().push(Fut::Inertial { key, gen, sig, res, sw, val });
                }
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
                store_elem: cb_store_elem,
                store_elem_part: cb_store_elem_part,
                nba_elem: cb_nba_elem,
                store_part_wide: cb_store_part_wide,
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
        if p >= self.nprocs {
            // an edge group: run every member (each from its wait), then wait again as a group
            // members run newest waiter first: they re-wait in the order they ran, so the order
            // alternates from one activation to the next (Icarus's LIFO waiting lists)
            let g = (p - self.nprocs) as usize;
            let n = self.groups[g].len();
            let rev = self.group_rev[g];
            for i in 0..n {
                let m = self.groups[g][if rev { n - 1 - i } else { i }];
                self.exec(m);
                if self.finished {
                    return;
                }
            }
            self.group_rev[g] = !rev;
            let pr = &mut self.procs[p as usize];
            pr.wait = NONE - 1 - g as u32;
            pr.wseq = self.hot.wseq;
            self.hot.wseq += 1;
            return;
        }
        self.stats.activations += 1;
        if self.time >= self.trace {
            eprintln!("{} proc {p} {} pc {}", self.time, self.prog.procs[p as usize].name, self.procs[p as usize].pc);
        }
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
                Op::Gen | Op::GenJ | Op::WResolve { .. } | Op::WCopy { .. } | Op::WAnd { .. } | Op::WOr { .. } | Op::WXor { .. } | Op::WNot { .. } | Op::WAdd { .. } | Op::WSub { .. } | Op::WEq { .. } | Op::WNe { .. } | Op::WCaseEq { .. } | Op::WCaseNe { .. } | Op::WLt { .. } | Op::WLe { .. } | Op::WSlice { .. } | Op::WResize { .. } | Op::WIns { .. } | Op::WMux { .. } | Op::WBool { .. } | Op::WStore { .. } | Op::WNba { .. } => {
                    if self.exec_gen(p, &orig[pc - 1], &mut pc) {
                        return;
                    }
                }
                Op::ArrRead1 { d, base, ix, depth } => {
                    let (e, x) = (m!(ix), m!(ix + 1));
                    if x == 0 && e < depth as u64 {
                        let src = base as usize + 2 * e as usize;
                        m!(d) = m!(src);
                        m!(d + 1) = m!(src + 1);
                    } else if self.exec_gen(p, &orig[pc - 1], &mut pc) {
                        return;
                    }
                }
                Op::DynSlice1 { d, s, ix, w, sw } => {
                    let (off, x) = (m!(ix), m!(ix + 1));
                    // (w <= sw, so the in-range test cannot overflow)
                    if x == 0 && off <= (sw - w) as u64 {
                        let mask = fast::mk(w);
                        m!(d) = (m!(s) >> off) & mask;
                        m!(d + 1) = (m!(s + 1) >> off) & mask;
                    } else if self.exec_gen(p, &orig[pc - 1], &mut pc) {
                        return;
                    }
                }
                Op::LNot1 { d, s } => {
                    let (a, b) = (m!(s), m!(s + 1));
                    let (v, x) = if a & !b != 0 {
                        (0, 0)
                    } else if b == 0 {
                        (1, 0)
                    } else {
                        (1, 1)
                    };
                    m!(d) = v;
                    m!(d + 1) = x;
                }
                Op::LtS1 { d, a, b, w } | Op::LeS1 { d, a, b, w } => {
                    if (m!(a + 1) | m!(b + 1)) != 0 {
                        m!(d) = 1;
                        m!(d + 1) = 1;
                    } else {
                        let (x, y) = (fast::sext(m!(a), w as u32) as i64, fast::sext(m!(b), w as u32) as i64);
                        m!(d) = if matches!(op, Op::LtS1 { .. }) { x < y } else { x <= y } as u64;
                        m!(d + 1) = 0;
                    }
                }
                Op::Resolve1 { d, a, b, w } => {
                    let (v, x) = fast::resolve1(m!(a), m!(a + 1), m!(b), m!(b + 1), fast::mk(w));
                    m!(d) = v;
                    m!(d + 1) = x;
                }
                Op::StorePart1 { sig, s, slot, lsb, w } => {
                    let mask = fast::mk(w) << lsb;
                    let (oa, ob) = (m!(slot), m!(slot + 1));
                    let na = (oa & !mask) | ((m!(s) << lsb) & mask);
                    let nb = (ob & !mask) | ((m!(s + 1) << lsb) & mask);
                    if na != oa || nb != ob {
                        m!(slot) = na;
                        m!(slot + 1) = nb;
                        self.notify(sig, ((oa & 1) | ((ob & 1) << 1)) as u8);
                    }
                }
                Op::StoreElem1 { sig, s, ix } => {
                    let (ea, eb, a, b) = (m!(ix), m!(ix + 1), m!(s), m!(s + 1));
                    self.store_elem(sig, ea, eb, a, b);
                }
                Op::StoreElemPart1 { sig, s, ix, lsb, w } => {
                    let (ea, eb, a, b) = (m!(ix), m!(ix + 1), m!(s), m!(s + 1));
                    self.store_elem_part(sig, ea, eb, lsb, w, a, b);
                }
                Op::NbaElem1 { sig, s, ix } => {
                    let (ea, eb, a, b) = (m!(ix), m!(ix + 1), m!(s), m!(s + 1));
                    self.nba_elem(sig, ea, eb, a, b);
                }
                Op::StorePartW { sig, s, slot, lsb, w, n } => {
                    let (a, b) = (m!(s), m!(s + 1));
                    self.store_part_wide(sig, slot, lsb, w, n, a, b);
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
                        // unknown condition: bits where both sides agree are kept
                        let (ta, tb, fa, fb) = (m!(t), m!(t + 1), m!(f), m!(f + 1));
                        let diff = (ta ^ fa) | (tb ^ fb);
                        m!(d) = ta | diff;
                        m!(d + 1) = tb | diff;
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
                    pr.wseq = self.hot.wseq;
                    self.hot.wseq += 1;
                    return;
                }
                Op::Delay { s } => {
                    let d = if m!(s + 1) == 0 { m!(s) } else { 0 };
                    self.delay(p, d);
                    self.procs[p as usize].pc = pc as u32;
                    return;
                }
                Op::Halt => {
                    let pr = &mut self.procs[p as usize];
                    pr.pc = (pc - 1) as u32;
                    pr.wait = NONE;
                    self.proc_done(p);
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
        if let Some(st) = &mut self.genstats {
            let k = format!("{ins:?}");
            let k: String = match ins {
                Instr::Store { t, mode, .. } => format!("Store {:?} {:?}", std::mem::discriminant(t), mode).chars().take(60).collect(),
                Instr::Bin { op, w, .. } => format!("Bin {op:?} w{w}"),
                Instr::Red { op, w, .. } => format!("Red {op:?} w{w}"),
                Instr::Cmp { op, w, .. } => format!("Cmp {op:?} w{w}"),
                _ => k.split(|c: char| c == ' ' || c == '{' || c == '(').next().unwrap_or("").to_string(),
            };
            *st.entry(k).or_default() += 1;
        }
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
                        // a real array reads 0.0 out of range (bits: all zero)
                        _ if self.prog.signals[*sig as usize].real => {
                            self.mem[*d as usize] = 0;
                            self.mem[*d as usize + 1] = 0;
                        }
                        _ => ops::set_x(&mut self.mem, *d as usize, r.width),
                    }
                }
                Instr::Store { t, s, mode } => {
                    // fast path: blocking whole-signal store of a ≤64-bit value
                    if let (Target::Whole { sig }, StoreMode::Blocking) = (t, mode) {
                        let r = self.sigs[*sig as usize];
                        // (forced signals go through `store`, which applies the force mask)
                        if r.width <= 64 && (self.force_mask.is_empty() || !self.force_mask.contains_key(sig)) {
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
                    self.delay(p, d);
                    self.procs[p as usize].pc = *pc as u32;
                    return true;
                }
                Instr::Wait { ctl } => {
                    let pr = &mut self.procs[p as usize];
                    pr.wait = *ctl;
                    pr.pc = *pc as u32;
                    pr.wseq = self.hot.wseq;
                    self.hot.wseq += 1;
                    return true;
                }
                Instr::Halt => {
                    let pr = &mut self.procs[p as usize];
                    pr.pc = (*pc - 1) as u32;
                    pr.wait = NONE;
                    self.proc_done(p);
                    return true;
                }
                Instr::Fork { children, join } => {
                    if self.fork(p, children, *join) {
                        let pr = &mut self.procs[p as usize];
                        pr.pc = *pc as u32;
                        pr.wait = NONE;
                        return true;
                    }
                }
                Instr::Udp { table, ins, prev, q } => {
                    let code = |m: &[u64], s: u32| -> u8 {
                        let s = s as usize;
                        match (m[s] & 1, m[s + 1] & 1) {
                            (0, 0) => UDP_0,
                            (1, 0) => UDP_1,
                            _ => UDP_X,
                        }
                    };
                    let cur: Vec<u8> = ins.iter().map(|&s| code(m, s)).collect();
                    let old: Vec<u8> = prev.iter().map(|&s| code(m, s)).collect();
                    let qv = code(m, *q);
                    let r = udp_eval(&self.prog.udps[*table as usize], &cur, &old, qv);
                    let (a, b) = match r {
                        UDP_0 => (0, 0),
                        UDP_1 => (1, 0),
                        _ => (1, 1),
                    };
                    let m = &mut self.mem;
                    m[*q as usize] = a;
                    m[*q as usize + 1] = b;
                    for (k, &s) in prev.iter().enumerate() {
                        let (a, b) = match cur[k] {
                            UDP_0 => (0, 0),
                            UDP_1 => (1, 0),
                            _ => (1, 1),
                        };
                        m[s as usize] = a;
                        m[s as usize + 1] = b;
                    }
                }
                Instr::Unforce { t } => {
                    if let Some((sig, res, _)) = self.resolve(t) {
                        if let Some((lsb, w)) = self.res_bits(sig, res) {
                            if let Some(m) = self.force_mask.get_mut(&sig) {
                                for b in lsb..lsb + w {
                                    m[(b / 64) as usize] &= !(1 << (b % 64));
                                }
                                if m.iter().all(|&x| x == 0) {
                                    self.force_mask.remove(&sig);
                                }
                            }
                        }
                    }
                }
                Instr::ResolveStr { d, w, drv } => ops::resolve_strength(m, *d as usize, *w, drv),
                Instr::Touch { sig } => {
                    let r = self.sigs[*sig as usize];
                    let cur = if r.depth == 0 { lsb_code(&self.mem, r.slot, r.width) } else { 0 };
                    self.notify(*sig, cur);
                }
                Instr::Kick { sig } => self.kick(*sig),
                Instr::SaveFrame { ranges } => {
                    // a runaway recursion stops the simulation instead of exhausting memory
                    let depth = self.frames.get(&p).map_or(0, |v| v.len());
                    let words: usize = ranges.iter().map(|r| r.1 as usize).sum();
                    if depth >= 100_000 || (depth + 1) * words > 64 << 20 {
                        let _ = writeln!(self.out, "ERROR: recursion too deep ({depth} calls) in {}", self.prog.procs[p as usize].name);
                        self.finished = true;
                        self.flush();
                        return true;
                    }
                    let saved: Vec<(u32, Vec<u64>)> =
                        ranges.iter().map(|&(lo, n)| (lo, self.mem[lo as usize..(lo + n) as usize].to_vec())).collect();
                    self.frames.entry(p).or_default().push(Frame { saved, ret: 0, dsts: Box::new([]) });
                }
                Instr::CallSub { entry, dsts } => {
                    if let Some(f) = self.frames.get_mut(&p).and_then(|v| v.last_mut()) {
                        f.ret = *pc as u32;
                        f.dsts = dsts.clone();
                    }
                    *pc = *entry as usize;
                }
                Instr::RetSub { srcs } => {
                    let Some(f) = self.frames.get_mut(&p).and_then(|v| v.pop()) else {
                        return true;
                    };
                    let vals: Vec<Vec<u64>> =
                        srcs.iter().map(|&(s, n)| self.mem[s as usize..(s + n) as usize].to_vec()).collect();
                    for (lo, v) in &f.saved {
                        self.mem[*lo as usize..*lo as usize + v.len()].copy_from_slice(v);
                    }
                    for (d, v) in f.dsts.iter().zip(&vals) {
                        self.mem[*d as usize..*d as usize + v.len()].copy_from_slice(v);
                    }
                    *pc = f.ret as usize;
                }
                Instr::Disable { target } => {
                    let (np, stopped) = self.disable(p, *target, *pc);
                    if stopped {
                        return true;
                    }
                    if let Some(np) = np {
                        *pc = np;
                    }
                }
                Instr::SysF { f, d } => {
                    let d = *d as usize;
                    match f {
                        SysFn::Time(e) => {
                            let div = 10u64.pow(*e as u32);
                            m[d] = (self.time + div / 2) / div;
                            m[d + 1] = 0;
                        }
                        SysFn::RealTime(e) => {
                            let v = self.time as f64 / ops::pow10(*e);
                            ops::wr_real(m, d, v);
                        }
                        SysFn::Random => {
                            let v = rtl_dist_uniform(&mut self.seed, i32::MIN, i32::MAX);
                            self.mem[d] = v as u32 as u64;
                            self.mem[d + 1] = 0;
                        }
                        SysFn::Urandom => {
                            let v = urandom(&mut self.useed, 0, u32::MAX);
                            self.mem[d] = v as u64;
                            self.mem[d + 1] = 0;
                        }
                        SysFn::UrandomRange(a, b) => {
                            let (a, b) = (self.mem[*a as usize] as u32, self.mem[*b as usize] as u32);
                            let v = urandom(&mut self.useed, a.min(b), a.max(b));
                            self.mem[d] = v as u64;
                            self.mem[d + 1] = 0;
                        }
                        SysFn::RandomSeed(sig) | SysFn::UrandomSeed(sig) => {
                            let r = self.sigs[*sig as usize];
                            let mut seed = self.mem[r.slot as usize] as u32 as i32;
                            let v = if matches!(f, SysFn::RandomSeed(_)) {
                                rtl_dist_uniform(&mut seed, i32::MIN, i32::MAX) as u32
                            } else {
                                // (Icarus: the shared $urandom state continues from this seed)
                                let v = urandom(&mut seed, 0, u32::MAX);
                                self.useed = seed;
                                v
                            };
                            self.mem[d] = v as u64;
                            self.mem[d + 1] = 0;
                            let tmp = [seed as u32 as u64 & ops::mask(r.width.min(64)), 0];
                            self.apply(*sig, Res::Whole, &tmp[..2.min(2 * nw(r.width))], r.width.min(64));
                        }
                    }
                }
                Instr::Clog2 { d, s, w } => ops::clog2(m, *d as usize, *s as usize, *w),
                Instr::IToR { .. }
                | Instr::RToI { .. }
                | Instr::RBin { .. }
                | Instr::RNeg { .. }
                | Instr::RCmp { .. }
                | Instr::RToTicks { .. } => {
                    ops::exec_real(ins, m);
                }
                Instr::Sys(task) => {
                    if self.systask(p, task, *pc) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Returns true if the process must stop executing.
    fn systask(&mut self, p: u32, task: &SysTask, pc: usize) -> bool {
        match task {
            SysTask::Display { nl, args, scope, fd, prefix, texp, radix } => {
                let mut s = String::from(*prefix);
                s.push_str(&fmt::format_args(args, &self.mem, scope, *radix, *texp));
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
                        } else if let Some(FileH::W(f)) = self.files.get_mut(&v) {
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
            SysTask::MonitorPrint { proc, args, scope, texp, radix } => {
                if self.monitor == *proc && self.monitor_on {
                    let mut s = fmt::format_args(args, &self.mem, scope, *radix, *texp);
                    s.push('\n');
                    let _ = self.out.write_all(s.as_bytes());
                }
            }
            SysTask::MonitorOn(on) => self.monitor_on = *on,
            SysTask::TimeFormat(f) => fmt::set_time_format(f.clone()),
            SysTask::Strobe { proc } => {
                self.procs[*proc as usize].pc = 0;
                self.postponed.push(*proc);
            }
            SysTask::Finish { loc, stop } => {
                if !loc.is_empty() {
                    let unit = match self.prog.precision.rem_euclid(3) {
                        0 => "1",
                        1 => "10",
                        _ => "100",
                    };
                    let base = match self.prog.precision.div_euclid(3) {
                        0 => "s",
                        -1 => "ms",
                        -2 => "us",
                        -3 => "ns",
                        -4 => "ps",
                        _ => "fs",
                    };
                    let what = if *stop { "$stop" } else { "$finish" };
                    let _ = writeln!(self.out, "{loc}: {what} called at {} ({unit}{base})", self.time);
                }
                self.finished = true;
                self.procs[p as usize].pc = pc as u32;
                self.flush();
                return true;
            }
            SysTask::DumpFile(f) => self.vcd_path = f.clone(),
            SysTask::DumpVars => {
                // (a trace started from the command line takes precedence)
                if self.vcd.is_none() {
                    let path = self.vcd_path.clone();
                    if self.trace_start(&path, &|_| true).is_ok() {
                        let _ = writeln!(self.out, "VCD info: dumpfile {path} opened for output.");
                    }
                }
            }
            SysTask::DumpOn(on) => {
                if self.vcd.is_some() {
                    self.dumping = *on;
                    self.track = *on || self.history.is_some();
                }
            }
            SysTask::ReadMem { hex, file, sig, start, end, loc } => {
                let start = start.and_then(|(s, w)| ops::read_u64(&self.mem, s as usize, w));
                let end = end.and_then(|(s, w)| ops::read_u64(&self.mem, s as usize, w));
                let file = self.file_name(file);
                self.readmem(*hex, &file, *sig, start, end, loc);
            }
            SysTask::WriteMem { hex, file, sig, start, end } => {
                let start = start.and_then(|(s, w)| ops::read_u64(&self.mem, s as usize, w));
                let end = end.and_then(|(s, w)| ops::read_u64(&self.mem, s as usize, w));
                let file = self.file_name(file);
                self.writemem(*hex, &file, *sig, start, end);
            }
            SysTask::TestPlusargs { d, prefix } => {
                let found = self.plusargs.iter().any(|a| a.starts_with(prefix.as_str()));
                self.mem[*d as usize] = found as u64;
                self.mem[*d as usize + 1] = 0;
            }
            SysTask::ValuePlusargs { d, prefix, conv, val, w, real } => {
                let found = self.plusargs.iter().find(|a| a.starts_with(prefix.as_str())).cloned();
                let (d, val) = (*d as usize, *val as usize);
                self.mem[d] = found.is_some() as u64;
                self.mem[d + 1] = 0;
                if let Some(a) = found {
                    plusarg_value(&mut self.mem, val, *w, *real, *conv, &a[prefix.len()..]);
                }
            }
            SysTask::SFormat { d, w, args, scope, texp, radix } => {
                let s = fmt::format_args(args, &self.mem, scope, *radix, *texp);
                string_to_bits(&mut self.mem, *d as usize, *w, &s);
            }
            SysTask::FOpen { d, dw, file, mode } => {
                let file = self.file_name(file);
                let opened: Option<FileH> = if mode.starts_with('r') {
                    std::fs::read(&file).ok().map(|data| FileH::R { data, pos: 0, eof: false })
                } else {
                    let mut o = std::fs::OpenOptions::new();
                    if mode.starts_with('a') {
                        o.append(true).create(true);
                    } else {
                        o.write(true).create(true).truncate(true);
                    }
                    o.open(&file).ok().map(|f| FileH::W(BufWriter::new(f)))
                };
                let fd = match opened {
                    Some(h) => {
                        let fd = 0x8000_0000 | self.next_fd;
                        self.next_fd += 1;
                        self.files.insert(fd, h);
                        fd
                    }
                    None => 0,
                };
                let tmp = [fd as u64 & ops::mask(*dw), 0];
                let d = *d as usize;
                self.mem[d] = tmp[0];
                self.mem[d + 1] = 0;
            }
            SysTask::FClose { s, loc } => {
                let v = self.mem[*s as usize] as u32;
                if v == 1 {
                    let _ = writeln!(self.out, "WARNING: {loc}: could not close MCD STDOUT (0x1) in $fclose().");
                }
                if let Some(FileH::W(mut f)) = self.files.remove(&v) {
                    let _ = f.flush();
                }
            }
            SysTask::FileRead { kind, d, src, scope } => {
                let r = self.file_read(kind, *src, scope);
                let d = *d as usize;
                self.mem[d] = r as u32 as u64;
                self.mem[d + 1] = 0;
            }
        }
        false
    }

    /// $fgetc & co.: the 32-bit result (-1 for EOF or errors).
    fn file_read(&mut self, kind: &FileRead, src: (u32, u32), scope: &str) -> i32 {
        let rd = |m: &[u64], s: u32| m[s as usize] as u32;
        if let FileRead::Scan { string: true, fmt, outs } = kind {
            let s = fmt::value_string(&self.mem, src.0 as usize, src.1);
            let mut pos = 0;
            let mut eof = false;
            return scan(&mut self.mem, s.as_bytes(), &mut pos, &mut eof, fmt, outs, scope);
        }
        let fd = rd(&self.mem, src.0);
        let Some(FileH::R { data, pos, eof }) = self.files.get_mut(&fd) else { return -1 };
        match kind {
            FileRead::Getc => match data.get(*pos) {
                Some(&c) => {
                    *pos += 1;
                    c as i32
                }
                None => {
                    *eof = true;
                    -1
                }
            },
            FileRead::Ungetc { c } => {
                let c = self.mem[*c as usize];
                if c > 255 {
                    return -1;
                }
                if *pos > 0 {
                    *pos -= 1;
                    data[*pos] = c as u8;
                } else {
                    data.insert(0, c as u8);
                }
                *eof = false;
                0
            }
            FileRead::Gets { val, w } => {
                let max = (*w / 8) as usize;
                let start = *pos;
                while *pos < data.len() && *pos - start < max {
                    *pos += 1;
                    if data[*pos - 1] == b'\n' {
                        break;
                    }
                }
                if *pos == start {
                    *eof = true;
                    return 0;
                }
                let line = String::from_utf8_lossy(&data[start..*pos]).into_owned();
                string_to_bits(&mut self.mem, *val as usize, *w, &line);
                (*pos - start) as i32
            }
            FileRead::Eof => *eof as i32,
            FileRead::Tell => *pos as i32,
            FileRead::Seek { off, op } => {
                let (off, op) = (self.mem[*off as usize] as u32 as i32 as i64, self.mem[*op as usize]);
                let base = match op {
                    0 => 0,
                    1 => *pos as i64,
                    2 => data.len() as i64,
                    _ => return -1,
                };
                let p = base + off;
                if p < 0 {
                    return -1;
                }
                *pos = p as usize;
                *eof = false;
                0
            }
            FileRead::Rewind => {
                *pos = 0;
                *eof = false;
                0
            }
            FileRead::Scan { fmt, outs, .. } => {
                let data = std::mem::take(data);
                let (mut p, mut e) = (*pos, *eof);
                let r = scan(&mut self.mem, &data, &mut p, &mut e, fmt, outs, scope);
                if let Some(FileH::R { data: d, pos, eof }) = self.files.get_mut(&fd) {
                    *d = data;
                    *pos = p;
                    *eof = e;
                }
                r
            }
        }
    }

    fn file_name(&self, f: &FileName) -> String {
        match f {
            FileName::Lit(s) => s.clone(),
            FileName::Val { s, w } => fmt::value_string(&self.mem, *s as usize, *w),
        }
    }

    /// Plusargs from the command line (without the leading '+').
    pub fn set_plusargs(&mut self, args: Vec<String>) {
        self.plusargs = args;
    }

    fn readmem(&mut self, hex: bool, file: &str, sig: u32, start: Option<u64>, end: Option<u64>, loc: &str) {
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
        let first = addr;
        let mut addressed = false;
        for tok in clean.split_whitespace() {
            if let Some(a) = tok.strip_prefix('@') {
                addr = i64::from_str_radix(a, 16).unwrap_or(addr);
                addressed = true;
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
        if !addressed && addr <= last {
            let name = if hex { "$readmemh" } else { "$readmemb" };
            let _ = writeln!(
                self.out,
                "WARNING: {loc}: {name}({file}): Not enough words in the file for the requested range [{first}:{last}]."
            );
        }
    }

    /// $writememh/$writememb: one word per line from `start` to `end` (either direction),
    /// after an address comment, like Icarus.
    fn writemem(&mut self, hex: bool, file: &str, sig: u32, start: Option<u64>, end: Option<u64>) {
        let r = self.sigs[sig as usize];
        let lo = self.prog.signals[sig as usize].arr_lo;
        let hi = lo + r.depth as i64 - 1;
        let from = start.map(|s| s as i64).unwrap_or(lo);
        let to = end.map(|e| e as i64).unwrap_or(hi);
        let n = nw(r.width);
        let mut out = String::from("// 0x00000000\n");
        let step = if to >= from { 1 } else { -1 };
        let mut a = from;
        loop {
            let e = a - lo;
            if e >= 0 && e < r.depth as i64 {
                let base = r.slot as usize + e as usize * 2 * n;
                let v = fmt::Val { a: &self.mem[base..base + n], b: &self.mem[base + n..base + 2 * n], w: r.width, signed: false };
                out.push_str(&fmt::fmt_radix(&v, if hex { 4 } else { 1 }, true));
                out.push('\n');
            }
            if a == to {
                break;
            }
            a += step;
        }
        if let Err(e) = std::fs::write(file, out) {
            eprintln!("warning: $writemem: cannot write '{file}': {e}");
        }
    }

    pub fn finished(&self) -> bool {
        self.finished
    }
}

/// IEEE 1364-2005 (17.9.3 / annex) `uniform`: the seed is a 32-bit long, as in Icarus.
fn uniform(seed: &mut i32, start: f64, end: f64) -> f64 {
    let d = 0.00000011920928955078125f64;
    if *seed == 0 {
        *seed = 259341593;
    }
    let (a, b) = if start >= end { (0.0, 2147483647.0) } else { (start, end) };
    *seed = seed.wrapping_mul(69069).wrapping_add(1);
    let st = ((*seed as u32) >> 9) | 0x3f800000;
    let c = f32::from_bits(st) as f64;
    let c = c + c * d;
    (b - a) * (c - 1.0) + a
}

/// `rtl_dist_uniform` of the standard ($random).
fn rtl_dist_uniform(seed: &mut i32, start: i32, end: i32) -> i32 {
    if start >= end {
        return start;
    }
    let floor = |r: f64| if r >= 0.0 { r as i64 } else { (r - 1.0) as i64 };
    if end != i32::MAX {
        let end = end as i64 + 1;
        let r = uniform(seed, start as f64, end as f64);
        floor(r).clamp(start as i64, end - 1) as i32
    } else if start != i32::MIN {
        let start = start as i64 - 1;
        let r = uniform(seed, start as f64, end as f64) + 1.0;
        floor(r).clamp(start + 1, end as i64) as i32
    } else {
        let r = (uniform(seed, start as f64, end as f64) + 2147483648.0) / 4294967295.0;
        let r = r * 4294967296.0 - 2147483648.0;
        floor(r) as i32
    }
}

/// $urandom / $urandom_range as Icarus computes them: uniform over [start, end + 1).
fn urandom(seed: &mut i32, start: u32, end: u32) -> u32 {
    if start >= end {
        return start;
    }
    let r = uniform(seed, start as f64, end as f64 + 1.0);
    (r as u64).min(end as u64) as u32
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
    let mut frames: Vec<Frame> = Vec::new();
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
            Instr::IToR { .. }
            | Instr::RToI { .. }
            | Instr::RBin { .. }
            | Instr::RNeg { .. }
            | Instr::RCmp { .. }
            | Instr::RToTicks { .. } => {
                ops::exec_real(ins, m);
            }
            Instr::Jmp { to } => pc = *to as usize,
            Instr::Jz { c, w, to } => {
                if ops::truth(m, *c as usize, *w) != Some(true) {
                    pc = *to as usize;
                }
            }
            Instr::SaveFrame { ranges } => {
                let saved: Vec<(u32, Vec<u64>)> =
                    ranges.iter().map(|&(lo, n)| (lo, m[lo as usize..(lo + n) as usize].to_vec())).collect();
                frames.push(Frame { saved, ret: 0, dsts: Box::new([]) });
                if frames.len() > 100_000 {
                    return Err("constant function recursion too deep".into());
                }
            }
            Instr::CallSub { entry, dsts } => {
                if let Some(f) = frames.last_mut() {
                    f.ret = pc as u32;
                    f.dsts = dsts.clone();
                }
                pc = *entry as usize;
            }
            Instr::RetSub { srcs } => {
                let Some(f) = frames.pop() else { break };
                let vals: Vec<Vec<u64>> = srcs.iter().map(|&(s, n)| m[s as usize..(s + n) as usize].to_vec()).collect();
                for (lo, v) in &f.saved {
                    m[*lo as usize..*lo as usize + v.len()].copy_from_slice(v);
                }
                for (d, v) in f.dsts.iter().zip(&vals) {
                    m[*d as usize..*d as usize + v.len()].copy_from_slice(v);
                }
                pc = f.ret as usize;
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
                Instr::Sys(t) if matches!(**t, SysTask::Finish { .. } | SysTask::Monitor { .. } | SysTask::Strobe { .. }) => {
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
                        | Target::ElemPart { sig, .. }
                        | Target::ElemDynPart { sig, .. } => (*sig, NONE),
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
unsafe extern "C" fn cb_store_elem(sim: *mut Sim, sig: u32, ea: u64, eb: u64, a: u64, b: u64) {
    (*sim).store_elem(sig, ea, eb, a, b);
}

#[cfg(feature = "jit")]
unsafe extern "C" fn cb_store_elem_part(sim: *mut Sim, sig: u32, ea: u64, eb: u64, lw: u32, a: u64, b: u64) {
    (*sim).store_elem_part(sig, ea, eb, lw as u8, (lw >> 8) as u8, a, b);
}

#[cfg(feature = "jit")]
unsafe extern "C" fn cb_nba_elem(sim: *mut Sim, sig: u32, ea: u64, eb: u64, a: u64, b: u64) {
    (*sim).nba_elem(sig, ea, eb, a, b);
}

#[cfg(feature = "jit")]
unsafe extern "C" fn cb_store_part_wide(sim: *mut Sim, sig: u32, slot: u32, lw: u32, n: u32, a: u64, b: u64) {
    (*sim).store_part_wide(sig, slot, lw >> 8, lw as u8, n as u8, a, b);
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
    sim.delay(p, d);
    sim.procs[p as usize].pc = pc;
}

#[cfg(feature = "jit")]
unsafe extern "C" fn cb_halt(sim: *mut Sim, p: u32, pc: u32) {
    let sim = &mut *sim;
    let pr = &mut sim.procs[p as usize];
    pr.pc = pc;
    pr.wait = NONE;
    sim.proc_done(p);
}

/// Convert a plusarg value for $value$plusargs.
/// An open file: a writer, or the whole contents of a file opened for reading.
enum FileH {
    W(BufWriter<std::fs::File>),
    R { data: Vec<u8>, pos: usize, eof: bool },
}

/// $fscanf/$sscanf over `inp` from `*pos`. Returns the number of conversions assigned, or -1
/// when the input ends before the first one.
fn scan(m: &mut [u64], inp: &[u8], pos: &mut usize, eof: &mut bool, fmt: &str, outs: &[ScanOut], scope: &str) -> i32 {
    for o in outs {
        m[o.ok as usize] = 0;
        m[o.ok as usize + 1] = 0;
    }
    let f = fmt.as_bytes();
    let (mut fi, mut n, mut oi) = (0usize, 0i32, 0usize);
    let ws = |c: u8| c.is_ascii_whitespace();
    let at_end = |p: usize, eof: &mut bool| {
        if p >= inp.len() {
            *eof = true;
            true
        } else {
            false
        }
    };
    let mut ret_eof = false;
    let pos0 = *pos;
    while fi < f.len() {
        let c = f[fi];
        if ws(c) {
            while fi < f.len() && ws(f[fi]) {
                fi += 1;
            }
            while *pos < inp.len() && ws(inp[*pos]) {
                *pos += 1;
            }
            continue;
        }
        if c != b'%' || f.get(fi + 1) == Some(&b'%') {
            if c == b'%' {
                fi += 1;
            }
            if at_end(*pos, eof) {
                ret_eof = n == 0 && *pos == pos0;
                break;
            }
            if inp[*pos] != c {
                break;
            }
            *pos += 1;
            fi += 1;
            continue;
        }
        // a conversion: %[*][width]conv
        fi += 1;
        let skip = f.get(fi) == Some(&b'*');
        if skip {
            fi += 1;
        }
        let mut width = 0usize;
        while fi < f.len() && f[fi].is_ascii_digit() {
            width = width * 10 + (f[fi] - b'0') as usize;
            fi += 1;
        }
        let Some(&conv) = f.get(fi) else { break };
        fi += 1;
        let conv = conv.to_ascii_lowercase();
        let width = if width == 0 { usize::MAX } else { width };
        if conv == b'm' {
            if !skip {
                if let Some(o) = outs.get(oi) {
                    string_to_bits(m, o.val as usize, o.w, scope);
                    m[o.ok as usize] = 1;
                    oi += 1;
                    n += 1;
                }
            }
            continue;
        }
        if conv != b'c' {
            while *pos < inp.len() && ws(inp[*pos]) {
                *pos += 1;
            }
        }
        if at_end(*pos, eof) {
            ret_eof = n == 0 && *pos == pos0;
            break;
        }
        let start = *pos;
        let take = |pos: &mut usize, ok: &dyn Fn(usize, u8) -> bool| {
            while *pos < inp.len() && *pos - start < width && ok(*pos - start, inp[*pos]) {
                *pos += 1;
            }
        };
        match conv {
            b'c' => *pos += 1,
            b's' => take(pos, &|_, c| !ws(c)),
            b'd' | b'u' => {
                let first = inp[start];
                if matches!(first, b'x' | b'X' | b'z' | b'Z' | b'?') {
                    *pos += 1;
                } else {
                    take(pos, &|i, c| c.is_ascii_digit() || c == b'_' || (i == 0 && (c == b'-' || c == b'+')));
                }
            }
            b'b' => take(pos, &|_, c| matches!(c, b'0' | b'1' | b'x' | b'X' | b'z' | b'Z' | b'?' | b'_')),
            b'o' => take(pos, &|_, c| matches!(c, b'0'..=b'7' | b'x' | b'X' | b'z' | b'Z' | b'?' | b'_')),
            b'h' | b'x' => take(pos, &|_, c| c.is_ascii_hexdigit() || matches!(c, b'x' | b'X' | b'z' | b'Z' | b'?' | b'_')),
            b'e' | b'f' | b'g' | b't' => {
                // [+-]digits[.digits][e[+-]digits]
                take(pos, &|_, c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.' | b'e' | b'E'));
            }
            _ => break,
        }
        let tok = &inp[start..*pos];
        let digits = tok.iter().any(|c| c.is_ascii_alphanumeric() || *c == b'?') || conv == b'c';
        if !digits {
            *pos = start;
            break;
        }
        if skip {
            continue;
        }
        let Some(o) = outs.get(oi) else { break };
        oi += 1;
        let text = String::from_utf8_lossy(tok).into_owned();
        let d = o.val as usize;
        match conv {
            b'c' => {
                if o.real {
                    ops::wr_real(m, d, tok[0] as f64);
                } else {
                    string_to_bits(m, d, o.w, &text);
                }
            }
            b's' => string_to_bits(m, d, o.w, &text),
            b'e' | b'f' | b'g' | b't' => {
                // the longest prefix that parses as a number
                let v = (1..=text.len()).rev().find_map(|k| text[..k].parse::<f64>().ok()).unwrap_or(0.0);
                if o.real {
                    ops::wr_real(m, d, v);
                } else {
                    ops::real_to_int(m, d, o.w, v);
                }
            }
            _ => {
                let radix = match conv {
                    b'b' => 1,
                    b'o' => 3,
                    b'h' | b'x' => 4,
                    _ => 0,
                };
                let text = text.replace('_', "");
                if o.real {
                    let v = if radix == 0 {
                        text.parse::<f64>().unwrap_or(0.0)
                    } else {
                        u64::from_str_radix(&text, 1 << radix).map(|v| v as f64).unwrap_or(0.0)
                    };
                    ops::wr_real(m, d, v);
                } else {
                    scan_int(m, d, o.w, radix, &text);
                }
            }
        }
        m[o.ok as usize] = 1;
        n += 1;
    }
    if ret_eof {
        -1
    } else {
        n
    }
}

/// An integer read by $scanf into the `w`-bit value at `d`: `bits` per digit (1, 3, 4), or
/// decimal (0). x/z digits set the unknown bits.
fn scan_int(m: &mut [u64], d: usize, w: u32, bits: u32, text: &str) {
    let n = nw(w);
    for i in 0..2 * n {
        m[d + i] = 0;
    }
    let set = |m: &mut [u64], bit: u32, a: bool, b: bool| {
        if bit < w {
            let (wi, sh) = ((bit / 64) as usize, bit % 64);
            m[d + wi] |= (a as u64) << sh;
            m[d + n + wi] |= (b as u64) << sh;
        }
    };
    if bits == 0 {
        let first = text.bytes().next().unwrap_or(b'0');
        if matches!(first, b'x' | b'X' | b'z' | b'Z' | b'?') {
            let z = first != b'x' && first != b'X';
            for bit in 0..w {
                set(m, bit, !z, true);
            }
            return;
        }
        let (neg, digits) = match text.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        // multiply-add over the value words
        for c in digits.bytes() {
            let mut carry = (c - b'0') as u128;
            for i in 0..n {
                let v = m[d + i] as u128 * 10 + carry;
                m[d + i] = v as u64;
                carry = v >> 64;
            }
        }
        if neg {
            let mut carry = 1u128;
            for i in 0..n {
                let v = (!m[d + i]) as u128 + carry;
                m[d + i] = v as u64;
                carry = v >> 64;
            }
        }
        m[d + n - 1] &= ops::mask(if w % 64 == 0 { 64 } else { w % 64 });
        return;
    }
    for (k, c) in text.bytes().rev().enumerate() {
        let (a, b) = match c {
            b'x' | b'X' => (u32::MAX, u32::MAX),
            b'z' | b'Z' | b'?' => (0, u32::MAX),
            _ => ((c as char).to_digit(16).unwrap_or(0), 0),
        };
        for j in 0..bits {
            set(m, k as u32 * bits + j, a >> j & 1 != 0, b >> j & 1 != 0);
        }
    }
}

fn plusarg_value(m: &mut [u64], d: usize, w: u32, real: bool, conv: char, text: &str) {
    let text = text.trim();
    match conv {
        'f' | 'e' | 'g' => {
            let v: f64 = text.parse().unwrap_or(0.0);
            if real {
                ops::wr_real(m, d, v);
            } else {
                ops::real_to_int(m, d, w, v);
            }
        }
        's' => string_to_bits(m, d, w, text),
        _ => {
            let radix = match conv {
                'h' | 'x' => 16,
                'o' => 8,
                'b' => 2,
                _ => 10,
            };
            let (neg, digits) = match text.strip_prefix('-') {
                Some(r) => (true, r),
                None => (false, text),
            };
            let v = i128::from_str_radix(digits, radix).unwrap_or(0);
            let v = if neg { -v } else { v };
            if real {
                ops::wr_real(m, d, v as f64);
            } else {
                ops::real_to_int(m, d, w, v as f64);
                if w <= 64 {
                    m[d] = (v as u64) & ops::mask(w);
                }
            }
        }
    }
}

/// Store a string as Verilog characters (last character in the low byte) in a w-bit value.
fn string_to_bits(m: &mut [u64], d: usize, w: u32, s: &str) {
    let n = nw(w);
    for i in 0..2 * n {
        m[d + i] = 0;
    }
    for (k, &b) in s.as_bytes().iter().rev().enumerate() {
        let bit = 8 * k as u32;
        if bit + 8 > w {
            break;
        }
        m[d + (bit / 64) as usize] |= (b as u64) << (bit % 64);
    }
}

/// Edge groups: clocked processes of the form `top: Wait ctl; body; Jmp top` (no other
/// suspension) with identical event-control triggers. When the triggers fire, all members run in
/// process order as one activation, which is the order they would have been woken in. Returns
/// (groups as (members, triggers), group of each process or NONE).
fn edge_groups(prog: &Program, ranks: &[u32]) -> (Vec<(Vec<u32>, Vec<(u32, Edge, u32)>)>, Vec<u32>) {
    let n = prog.procs.len();
    let mut by_trig: HashMap<Vec<(u32, u8, u32)>, Vec<u32>> = HashMap::new();
    let mut order: Vec<Vec<(u32, u8, u32)>> = Vec::new();
    for p in 0..n {
        let pd = &prog.procs[p];
        if pd.class != ProcClass::Normal || ranks[p] != NONE || pd.code.len() < 2 {
            continue;
        }
        let ctl = match (&pd.code[0], &pd.code[pd.code.len() - 1]) {
            (Instr::Wait { ctl }, Instr::Jmp { to: 0 }) => *ctl,
            _ => continue,
        };
        let simple = pd.code[1..].iter().all(|i| !matches!(i, Instr::Wait { .. } | Instr::Delay { .. } | Instr::Halt));
        let c = &prog.ctls[ctl as usize];
        if !simple || c.triggers.is_empty() || c.triggers.iter().all(|t| t.1 == Edge::Any) {
            continue;
        }
        let key: Vec<(u32, u8, u32)> = c.triggers.iter().map(|t| (t.0, t.1 as u8, t.2)).collect();
        let e = by_trig.entry(key.clone()).or_default();
        if e.is_empty() {
            order.push(key);
        }
        e.push(p as u32);
    }
    let mut groups = Vec::new();
    let mut group_of = vec![NONE; n];
    for key in order {
        let members = by_trig.remove(&key).unwrap();
        if members.len() < 2 {
            continue;
        }
        let g = groups.len() as u32;
        for &m in &members {
            group_of[m as usize] = g;
        }
        let first = &prog.ctls[match prog.procs[members[0] as usize].code[0] {
            Instr::Wait { ctl } => ctl as usize,
            _ => unreachable!(),
        }];
        groups.push((members, first.triggers.clone()));
    }
    (groups, group_of)
}

/// Evaluate a UDP table: level rows first, then (for a changed input) edge rows. No matching row
/// gives x (combinational, or a sequential UDP whose inputs changed); otherwise the state stays.
fn udp_eval(t: &UdpTable, cur: &[u8], prev: &[u8], q: u8) -> u8 {
    let level_ok = |row: &UdpRow| row.ins.iter().zip(cur).all(|(s, &v)| s.edge || s.cur & v != 0);
    let state_ok = |row: &UdpRow| !t.sequential || row.state & q != 0;
    let result = |row: &UdpRow| -> u8 {
        match row.out {
            0 => UDP_0,
            1 => UDP_1,
            3 => q,
            _ => UDP_X,
        }
    };
    for row in &t.rows {
        if !row.has_edge && level_ok(row) && state_ok(row) {
            return result(row);
        }
    }
    let changed = cur.iter().zip(prev).any(|(a, b)| a != b);
    if t.sequential && changed {
        for row in &t.rows {
            if !row.has_edge || !state_ok(row) {
                continue;
            }
            let ok = row.ins.iter().enumerate().all(|(k, s)| {
                if s.edge {
                    prev[k] != cur[k] && s.prev & prev[k] != 0 && s.cur & cur[k] != 0
                } else {
                    s.cur & cur[k] != 0
                }
            });
            if ok {
                return result(row);
            }
        }
    }
    if t.sequential && !changed {
        q
    } else {
        UDP_X
    }
}
