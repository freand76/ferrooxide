//! VCD waveform writer ($dumpfile / $dumpvars).
//! Signals are recorded when they change (a dirty list collected by the scheduler) and written at
//! the end of each time step.

use crate::{nw, Program, SigKind};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};

pub struct Vcd {
    out: BufWriter<File>,
    /// per signal: VCD identifier codes (several names may alias one signal)
    pub ids: Vec<Vec<String>>,
    last_time: Option<u64>,
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
        let f = File::create(path)?;
        let mut out = BufWriter::with_capacity(1 << 16, f);
        let mut ids = vec![Vec::new(); prog.signals.len()];
        let mut root = Scope::default();
        let mut next = 0usize;
        let mut sig_code: Vec<Option<String>> = vec![None; prog.signals.len()];
        for (name, sig) in &prog.names {
            let info = &prog.signals[*sig as usize];
            if info.depth > 0 {
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
        writeln!(out, "$timescale\n\t1s\n$end")?;
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
        Ok(Vcd { out, ids, last_time: None, enabled: true })
    }

    pub fn write_value(&mut self, sig: u32, mem: &[u64], slot: u32, w: u32) {
        let codes = &self.ids[sig as usize];
        if codes.is_empty() {
            return;
        }
        let n = nw(w);
        let s = slot as usize;
        let bit = |i: u32| -> char {
            let wi = (i / 64) as usize;
            let a = (mem[s + wi] >> (i % 64)) & 1;
            let b = (mem[s + n + wi] >> (i % 64)) & 1;
            match (a, b) {
                (0, 0) => '0',
                (1, 0) => '1',
                (0, 1) => 'z',
                _ => 'x',
            }
        };
        for code in codes {
            if w == 1 {
                let _ = writeln!(self.out, "{}{}", bit(0), code);
            } else {
                let mut st = String::with_capacity(w as usize + 8);
                st.push('b');
                // strip leading zeros (VCD convention), keep at least one digit
                let mut started = false;
                for i in (0..w).rev() {
                    let c = bit(i);
                    if !started && c == '0' && i != 0 {
                        continue;
                    }
                    started = true;
                    st.push(c);
                }
                let _ = writeln!(self.out, "{} {}", st, code);
            }
        }
    }

    pub fn time(&mut self, t: u64) {
        if self.last_time != Some(t) {
            let _ = writeln!(self.out, "#{t}");
            self.last_time = Some(t);
        }
    }

    pub fn begin_dumpvars(&mut self) {
        let _ = writeln!(self.out, "$dumpvars");
    }
    pub fn end_dumpvars(&mut self) {
        let _ = writeln!(self.out, "$end");
    }

    pub fn flush(&mut self) {
        let _ = self.out.flush();
    }
}
