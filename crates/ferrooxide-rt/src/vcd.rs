//! VCD waveform writer ($dumpfile / $dumpvars, --vcd/--trace).
//! Signals are recorded when they change (a dirty list collected by the scheduler) and written at
//! the end of each time step. As in DigSim's writer: a value is written only when it differs from
//! the last one written for the signal, and a timestamp only before the first value at that time.

use crate::{nw, Program, SigKind};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};

pub struct Vcd {
    out: BufWriter<File>,
    /// per signal: VCD identifier codes (several names may alias one signal)
    pub ids: Vec<Vec<String>>,
    /// per traced signal: the planes last written (empty: nothing written yet)
    last: Vec<Vec<u64>>,
    last_time: Option<u64>,
    /// the time of the next value written (its "#t" line is written with it)
    time: u64,
    line: Vec<u8>,
    pub enabled: bool,
}

fn ident(mut v: usize) -> String {
    let mut s = String::new();
    loop {
        s.push((b'!' + (v % 94) as u8) as char);
        v /= 94;
        if v == 0 {
            break;
        }
    }
    s
}

#[derive(Default)]
struct Scope {
    vars: Vec<(String, u32, String)>,
    subs: BTreeMap<String, Scope>,
}

impl Vcd {
    pub fn create(path: &str, prog: &Program) -> std::io::Result<Vcd> {
        Self::create_filtered(path, prog, &|_| true)
    }

    /// A VCD of the signals whose hierarchical name `keep` accepts.
    pub fn create_filtered(path: &str, prog: &Program, keep: &dyn Fn(&str) -> bool) -> std::io::Result<Vcd> {
        let f = File::create(path)?;
        let mut out = BufWriter::with_capacity(1 << 16, f);
        let mut ids = vec![Vec::new(); prog.signals.len()];
        let mut root = Scope::default();
        let mut next = 0usize;
        let mut sig_code: Vec<Option<String>> = vec![None; prog.signals.len()];
        for (name, sig) in &prog.names {
            let info = &prog.signals[*sig as usize];
            if info.depth > 0 || !keep(name) {
                continue;
            }
            let code = match &sig_code[*sig as usize] {
                Some(c) => c.clone(),
                None => {
                    let c = ident(next);
                    next += 1;
                    ids[*sig as usize].push(c.clone());
                    sig_code[*sig as usize] = Some(c.clone());
                    c
                }
            };
            let parts: Vec<&str> = name.split('.').collect();
            let mut sc = &mut root;
            for p in &parts[..parts.len() - 1] {
                sc = sc.subs.entry(p.to_string()).or_default();
            }
            sc.vars.push((parts[parts.len() - 1].to_string(), *sig, code));
        }
        writeln!(out, "$date\n\tferrooxide\n$end\n$version\n\tferrooxide {}\n$end", env!("CARGO_PKG_VERSION"))?;
        // times are in simulation ticks: 10^precision s
        let p = prog.precision;
        let unit = ["s", "ms", "us", "ns", "ps", "fs"][(-p.div_euclid(3)).clamp(0, 5) as usize];
        writeln!(out, "$timescale\n\t{}{unit}\n$end", ["1", "10", "100"][p.rem_euclid(3) as usize])?;
        fn emit(out: &mut BufWriter<File>, sc: &Scope, prog: &Program) -> std::io::Result<()> {
            for (n, s, code) in &sc.vars {
                let info = &prog.signals[*s as usize];
                let kind = match info.kind {
                    SigKind::Wire => "wire",
                    SigKind::Reg => "reg",
                    SigKind::Integer => "integer",
                };
                if info.width == 1 && info.msb == 0 && info.lsb == 0 && info.kind != SigKind::Integer {
                    writeln!(out, "$var {kind} 1 {code} {n} $end")?;
                } else {
                    writeln!(out, "$var {kind} {} {code} {n} [{}:{}] $end", info.width, info.msb, info.lsb)?;
                }
            }
            for (name, sub) in &sc.subs {
                writeln!(out, "$scope module {name} $end")?;
                emit(out, sub, prog)?;
                writeln!(out, "$upscope $end")?;
            }
            Ok(())
        }
        emit(&mut out, &root, prog)?;
        writeln!(out, "$enddefinitions $end")?;
        let last = vec![Vec::new(); ids.len()];
        Ok(Vcd { out, ids, last, last_time: None, time: 0, line: Vec::new(), enabled: true })
    }

    pub fn write_value(&mut self, sig: u32, mem: &[u64], slot: u32, w: u32) {
        let codes = &self.ids[sig as usize];
        if codes.is_empty() {
            return;
        }
        let n = nw(w);
        let s = slot as usize;
        let cur = &mem[s..s + 2 * n];
        let last = &mut self.last[sig as usize];
        if last.as_slice() == cur {
            return; // changed and changed back within the time step
        }
        last.clear();
        last.extend_from_slice(cur);
        if self.last_time != Some(self.time) {
            let _ = writeln!(self.out, "#{}", self.time);
            self.last_time = Some(self.time);
        }
        let bit = |i: u32| -> u8 {
            let wi = (i / 64) as usize;
            let a = (mem[s + wi] >> (i % 64)) & 1;
            let b = (mem[s + n + wi] >> (i % 64)) & 1;
            match (a, b) {
                (0, 0) => b'0',
                (1, 0) => b'1',
                (0, 1) => b'z',
                _ => b'x',
            }
        };
        let line = &mut self.line;
        line.clear();
        if w == 1 {
            line.push(bit(0));
        } else {
            line.push(b'b');
            // shortest form (VCD left-extends 0, x and z): a leading run of x or z becomes one
            // character, leading zeros go when a 1 follows
            let first = bit(w - 1);
            let mut i = w - 1;
            match first {
                b'x' | b'z' => {
                    while i > 0 && bit(i - 1) == first {
                        i -= 1;
                    }
                }
                b'0' => {
                    while i > 0 && bit(i) == b'0' && bit(i - 1) != b'x' && bit(i - 1) != b'z' {
                        i -= 1;
                    }
                }
                _ => {}
            }
            for k in (0..=i).rev() {
                line.push(bit(k));
            }
            line.push(b' ');
        }
        let k = line.len();
        for code in codes {
            line.truncate(k);
            line.extend_from_slice(code.as_bytes());
            line.push(b'\n');
            let _ = self.out.write_all(line);
        }
    }

    /// The time of the values written next.
    pub fn time(&mut self, t: u64) {
        self.time = t;
    }

    /// Forget the last values: the next write of every signal is a full dump.
    pub fn reset_values(&mut self) {
        for l in &mut self.last {
            l.clear();
        }
    }

    pub fn begin_dumpvars(&mut self) {
        if self.last_time != Some(self.time) {
            let _ = writeln!(self.out, "#{}", self.time);
            self.last_time = Some(self.time);
        }
        let _ = writeln!(self.out, "$dumpvars");
    }
    pub fn end_dumpvars(&mut self) {
        let _ = writeln!(self.out, "$end");
    }

    pub fn flush(&mut self) {
        let _ = self.out.flush();
    }
}
