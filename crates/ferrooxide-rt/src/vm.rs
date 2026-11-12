//! The event scheduler (IEEE 1364 stratified regions) and the bytecode interpreter.

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
    fan_lo: u32,
    fan_hi: u32,
}

#[derive(Clone, Copy)]
struct Fan {
    proc: u32,
    ctl: u32,
    edge: Edge,
}

#[derive(Clone, Copy)]
struct ProcRt {
    pc: u32,
    wait: u32,
    class: ProcClass,
    started: bool,
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
    off: u32,
    sw: u32,
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
    code: Rc<Vec<Vec<Instr>>>,
    prog: Program,
    pub mem: Vec<u64>,
    sigs: Vec<SigRt>,
    fan: Vec<Fan>,
    procs: Vec<ProcRt>,
    active: VecDeque<u32>,
    inactive: Vec<u32>,
    nba: Vec<Nba>,
    nba_vals: Vec<u64>,
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
    pub fn new(prog: Program) -> Sim {
        let nsig = prog.signals.len();
        // build fanout table
        let mut per_sig: Vec<Vec<Fan>> = vec![Vec::new(); nsig];
        for (ci, c) in prog.ctls.iter().enumerate() {
            for &(sig, edge) in &c.triggers {
                per_sig[sig as usize].push(Fan { proc: c.proc, ctl: ci as u32, edge });
            }
        }
        let mut fan = Vec::new();
        let mut sigs = Vec::with_capacity(nsig);
        for (i, s) in prog.signals.iter().enumerate() {
            let lo = fan.len() as u32;
            fan.extend_from_slice(&per_sig[i]);
            sigs.push(SigRt { slot: s.slot, width: s.width, depth: s.depth, fan_lo: lo, fan_hi: fan.len() as u32 });
        }
        let procs = prog
            .procs
            .iter()
            .map(|p| ProcRt { pc: 0, wait: NONE, class: p.class, started: false })
            .collect();
        let code = Rc::new(prog.procs.iter().map(|p| p.code.clone()).collect());
        Sim {
            code,
            mem: prog.mem.clone(),
            sigs,
            fan,
            procs,
            active: VecDeque::new(),
            inactive: Vec::new(),
            nba: Vec::new(),
            nba_vals: Vec::new(),
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
            prog,
        }
    }

    /// Run until no events remain, $finish, or `max_time` is passed.
    pub fn run(&mut self, max_time: Option<u64>) {
        for (i, p) in self.procs.iter_mut().enumerate() {
            if p.class == ProcClass::Normal {
                p.started = true;
                self.active.push_back(i as u32);
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
                    Fut::Wake(p) => self.active.push_back(p),
                    Fut::Nba { sig, res, sw, val } => {
                        let off = self.nba_vals.len() as u32;
                        self.nba_vals.extend_from_slice(&val);
                        self.nba.push(Nba { sig, res, off, sw });
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
            while let Some(p) = self.active.pop_front() {
                self.exec(p);
                if self.finished {
                    return;
                }
            }
            if !self.inactive.is_empty() {
                self.active.extend(self.inactive.drain(..));
                continue;
            }
            if !self.nba.is_empty() {
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

    fn apply_nba(&mut self) {
        let list = std::mem::take(&mut self.nba);
        let vals = std::mem::take(&mut self.nba_vals);
        self.stats.nba_updates += list.len() as u64;
        for e in &list {
            let n = 2 * nw(e.sw);
            let o = e.off as usize;
            self.apply(e.sig, e.res, &vals[o..o + n], e.sw);
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
        if self.dumping && !self.is_dirty[sig as usize] {
            self.is_dirty[sig as usize] = true;
            self.dirty.push(sig);
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
            if pr.class == ProcClass::Postponed {
                self.postponed.push(f.proc);
            } else {
                self.active.push_back(f.proc);
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
            self.notify(sig, old);
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
                let off = self.nba_vals.len() as u32;
                self.nba_vals.extend_from_slice(&self.mem[s..s + n]);
                self.nba.push(Nba { sig, res, off, sw });
            }
            StoreMode::NbaDelay(ds) => {
                let d = ops::read_u64(&self.mem, ds as usize, 64).unwrap_or(0);
                let val: Box<[u64]> = self.mem[s..s + n].into();
                if d == 0 {
                    let off = self.nba_vals.len() as u32;
                    self.nba_vals.extend_from_slice(&val);
                    self.nba.push(Nba { sig, res, off, sw });
                } else {
                    self.future.entry(self.time + d).or_default().push(Fut::Nba { sig, res, sw, val });
                }
            }
        }
    }

    /// Execute process `p` until it suspends.
    fn exec(&mut self, p: u32) {
        self.stats.activations += 1;
        let code_rc = self.code.clone();
        let code = &code_rc[p as usize];
        let mut pc = self.procs[p as usize].pc as usize;
        loop {
            let ins = &code[pc];
            pc += 1;
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
                            continue;
                        }
                    }
                    self.store(t, *s, *mode);
                }
                Instr::Jmp { to } => pc = *to as usize,
                Instr::Jz { c, w, to } => {
                    if ops::truth(m, *c as usize, *w) != Some(true) {
                        pc = *to as usize;
                    }
                }
                Instr::Delay { s } => {
                    let d = ops::read_u64(m, *s as usize, 64).unwrap_or(0);
                    if d == 0 {
                        self.inactive.push(p);
                    } else {
                        self.future.entry(self.time + d).or_default().push(Fut::Wake(p));
                    }
                    self.procs[p as usize].pc = pc as u32;
                    return;
                }
                Instr::Wait { ctl } => {
                    let pr = &mut self.procs[p as usize];
                    pr.wait = *ctl;
                    pr.pc = pc as u32;
                    return;
                }
                Instr::Halt => {
                    let pr = &mut self.procs[p as usize];
                    pr.pc = (pc - 1) as u32;
                    pr.wait = NONE;
                    return;
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
                    if self.systask(p, task, pc) {
                        return;
                    }
                }
            }
        }
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
