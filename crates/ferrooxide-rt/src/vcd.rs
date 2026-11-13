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
    /// per traced signal: offset of the planes last written in `last_buf` (u32::MAX: not
    /// traced), and whether anything was written yet
    last_off: Vec<u32>,
    last_buf: Vec<u64>,
    written: Vec<bool>,
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
        // (as Icarus: the parameter block, which is empty here)
        writeln!(out, "$comment Show the parameter values. $end\n$dumpall\n$end")?;
        let mut last_off = vec![u32::MAX; ids.len()];
        let mut last_buf = Vec::new();
        for (sig, codes) in ids.iter().enumerate() {
            if !codes.is_empty() {
                last_off[sig] = last_buf.len() as u32;
                last_buf.resize(last_buf.len() + 2 * nw(prog.signals[sig].width), 0);
            }
        }
        let written = vec![false; ids.len()];
        Ok(Vcd { out, ids, last_off, last_buf, written, last_time: None, time: 0, line: Vec::new(), enabled: true })
    }

    pub fn write_value(&mut self, sig: u32, mem: &[u64], slot: u32, w: u32) {
        let codes = &self.ids[sig as usize];
        if codes.is_empty() {
            return;
        }
        let n = nw(w);
        let s = slot as usize;
        let cur = &mem[s..s + 2 * n];
        let off = self.last_off[sig as usize] as usize;
        let last = &mut self.last_buf[off..off + 2 * n];
        let same = self.written[sig as usize] && last.iter().zip(cur).all(|(a, b)| a == b);
        if same {
            return; // changed and changed back within the time step
        }
        for (d, v) in last.iter_mut().zip(cur) {
            *d = *v;
        }
        self.written[sig as usize] = true;
        if self.last_time != Some(self.time) {
            let _ = writeln!(self.out, "#{}", self.time);
            self.last_time = Some(self.time);
        }
        let (va, vb) = (&cur[..n], &cur[n..]);
        let line = &mut self.line;
        line.clear();
        if w == 1 {
            line.push(match (va[0] & 1, vb[0] & 1) {
                (0, 0) => b'0',
                (1, 0) => b'1',
                (0, _) => b'z',
                _ => b'x',
            });
        } else {
            line.push(b'b');
            render_vector(line, va, vb, w);
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

    /// The end of the simulation: its time is written even without changes.
    pub fn end(&mut self, t: u64) {
        if self.last_time != Some(t) {
            let _ = writeln!(self.out, "#{t}");
            self.last_time = Some(t);
        }
    }

    /// Forget the last values: the next write of every signal is a full dump.
    pub fn reset_values(&mut self) {
        self.written.iter_mut().for_each(|w| *w = false);
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

/// "0"/"1" for the 8 bits of a byte, most significant first.
static BYTE_BITS: [[u8; 8]; 256] = {
    let mut t = [[0u8; 8]; 256];
    let mut v = 0;
    while v < 256 {
        let mut k = 0;
        while k < 8 {
            t[v][k] = if (v >> (7 - k)) & 1 == 1 { b'1' } else { b'0' };
            k += 1;
        }
        v += 1;
    }
    t
};

/// The shortest VCD text of a vector (VCD left-extends 0, x and z): a leading run of x or z
/// becomes one character, leading zeros go when a 1 follows.
fn render_vector(line: &mut Vec<u8>, a: &[u64], b: &[u64], w: u32) {
    if b.iter().all(|&x| x == 0) {
        // known bits: from the highest 1, eight at a time
        let Some(top) = (0..a.len()).rev().find(|&k| a[k] != 0) else {
            line.push(b'0');
            return;
        };
        let hi = top as u32 * 64 + 63 - a[top].leading_zeros();
        let mut i = hi as i64;
        while i >= 0 && (i + 1) % 8 != 0 {
            line.push(b'0' + ((a[(i / 64) as usize] >> (i % 64)) & 1) as u8);
            i -= 1;
        }
        while i >= 7 {
            let byte = (a[(i / 64) as usize] >> ((i - 7) % 64)) as u8;
            line.extend_from_slice(&BYTE_BITS[byte as usize]);
            i -= 8;
        }
        return;
    }
    let start = line.len();
    for i in (0..w).rev() {
        let (wi, sh) = ((i / 64) as usize, i % 64);
        line.push(match ((a[wi] >> sh) & 1, (b[wi] >> sh) & 1) {
            (0, 0) => b'0',
            (1, 0) => b'1',
            (0, _) => b'z',
            _ => b'x',
        });
    }
    // compress the leading run
    let first = line[start];
    let mut k = start;
    let end = line.len();
    match first {
        b'x' | b'z' => {
            while k + 1 < end && line[k + 1] == first {
                k += 1;
            }
        }
        _ => {
            // leading zeros (a 1, x or z follows: x and z must keep a 0 in front)
            while k + 1 < end && line[k] == b'0' && line[k + 1] == b'0' {
                k += 1;
            }
            if k + 1 < end && line[k] == b'0' && line[k + 1] == b'1' {
                k += 1;
            }
        }
    }
    line.drain(start..k);
}
