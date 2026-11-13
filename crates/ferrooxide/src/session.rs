//! A simulation session: load a design, run it, inspect and change signals, trace. Shared by the
//! interactive mode (`-i`) and the MCP server (`--mcp`); every operation returns its report as
//! text (errors as `Err`).

use ferrooxide_rt::{RunState, Sim};
use std::path::PathBuf;
use std::time::Instant;

/// What to load and how.
#[derive(Clone, Default)]
pub struct LoadOptions {
    pub files: Vec<PathBuf>,
    pub defines: Vec<(String, String)>,
    pub incdirs: Vec<PathBuf>,
    pub tops: Vec<String>,
    pub plusargs: Vec<String>,
    pub dialect: ferrooxide_compile::Dialect,
    pub jit: bool,
    pub fuse: bool,
}

pub struct Session {
    pub sim: Sim,
    precision: i32,
    started: bool,
    finished: bool,
    /// watched signals: (name, signal, radix, last value)
    watches: Vec<(String, u32, char, String)>,
    /// breakpoints: (name, signal, value to stop at (None: any change), last value)
    breaks: Vec<(String, u32, Option<Vec<u64>>, String)>,
    /// what loading said: the summary line and warnings
    pub load_report: String,
}

/// Parse, elaborate and prepare a design. Every signal can be forced (interactive use).
pub fn load(o: &LoadOptions) -> Result<Session, String> {
    let mut opts = ferrooxide_front::Options::default();
    for (k, v) in &o.defines {
        opts.defines.insert(k.clone(), v.clone());
    }
    opts.incdirs = o.incdirs.clone();
    let t0 = Instant::now();
    let modules = ferrooxide_front::parse_files(&o.files, &opts).map_err(|e| e.to_string())?;
    let t1 = Instant::now();
    let (mut prog, warnings, _) =
        ferrooxide_compile::compile_dialect(&modules, &o.tops, false, o.dialect).map_err(|e| e.to_string())?;
    let t2 = Instant::now();
    prog.no_fuse = !o.fuse;
    for s in prog.signals.iter_mut() {
        if s.depth == 0 {
            s.forced = true;
        }
    }
    let (nsig, nproc, precision) = (prog.signals.len(), prog.procs.len(), prog.precision);
    let mut sim = Sim::new(prog);
    sim.set_plusargs(o.plusargs.clone());
    let mut report = String::new();
    for w in &warnings {
        report.push_str(w);
        report.push('\n');
    }
    if o.jit {
        if let Err(e) = sim.enable_jit() {
            report.push_str(&format!("JIT compilation failed ({e}), interpreting\n"));
        }
    }
    let t3 = Instant::now();
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
    report.push_str(&format!(
        "loaded {} files, {} modules: {nsig} signals, {nproc} processes; parse {:.0} ms, elaborate {:.0} ms, compile {:.0} ms; time unit {}",
        o.files.len(),
        modules.len(),
        ms(t1 - t0),
        ms(t2 - t1),
        ms(t3 - t2),
        unit_name(precision)
    ));
    Ok(Session {
        sim,
        precision,
        started: false,
        finished: false,
        watches: Vec::new(),
        breaks: Vec::new(),
        load_report: report,
    })
}

/// The lint check of a design (incomplete sensitivity lists), as text.
pub fn lint(o: &LoadOptions) -> Result<String, String> {
    let mut opts = ferrooxide_front::Options::default();
    for (k, v) in &o.defines {
        opts.defines.insert(k.clone(), v.clone());
    }
    opts.incdirs = o.incdirs.clone();
    let modules = ferrooxide_front::parse_files(&o.files, &opts).map_err(|e| e.to_string())?;
    let (_, _, findings) =
        ferrooxide_compile::compile_dialect(&modules, &o.tops, true, o.dialect).map_err(|e| e.to_string())?;
    let mut out = findings.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!("{} finding{}", findings.len(), if findings.len() == 1 { "" } else { "s" }));
    Ok(out)
}

fn unit_name(p: i32) -> String {
    let base = ["s", "ms", "us", "ns", "ps", "fs"][(-p.div_euclid(3)).clamp(0, 5) as usize];
    format!("{}{base}", ["1", "10", "100"][p.rem_euclid(3) as usize])
}

impl Session {
    /// A session for a simulator prepared elsewhere (signals made forceable by the caller).
    pub fn from_sim(sim: Sim, precision: i32, load_report: String) -> Session {
        Session { sim, precision, started: false, finished: false, watches: Vec::new(), breaks: Vec::new(), load_report }
    }

    fn ensure_started(&mut self) {
        if !self.started {
            self.started = true;
            self.sim.start();
        }
    }

    pub fn time(&self) -> u64 {
        self.sim.now()
    }

    pub fn status(&self) -> String {
        format!(
            "time {} ({} units of {}){}; {} watches, {} breakpoints, {} signals traced",
            self.sim.now(),
            self.sim.now(),
            unit_name(self.precision),
            if self.finished { ", $finish called" } else { "" },
            self.watches.len(),
            self.breaks.len(),
            self.sim.traced_signals()
        )
    }

    /// Run to `until` (absolute ticks; None: until nothing is scheduled). Watch changes and
    /// breakpoint hits are reported line by line, then the stop reason.
    pub fn run(&mut self, until: Option<u64>) -> String {
        self.ensure_started();
        if self.finished {
            return "the simulation has finished ($finish)".into();
        }
        let mut report = String::new();
        let st = if self.watches.is_empty() && self.breaks.is_empty() {
            self.sim.advance(until)
        } else {
            self.run_watched(until, &mut report)
        };
        self.sim.flush();
        match st {
            RunState::Paused => report.push_str(&format!("stopped at time {}", self.sim.now())),
            RunState::Idle => report.push_str(&format!("time {}: nothing more is scheduled", self.sim.now())),
            RunState::Finished => {
                self.finished = true;
                report.push_str(&format!("time {}: $finish", self.sim.now()));
            }
        }
        report
    }

    /// The current and the next time step.
    pub fn step(&mut self) -> String {
        self.ensure_started();
        let next = self.sim.next_event_time().unwrap_or(self.sim.now());
        self.run(Some(next))
    }

    /// A time argument: ticks, `+T` relative, or with a unit (10ns, 2.5us).
    pub fn parse_until(&self, t: &str) -> Result<u64, String> {
        let v = parse_time(t.trim_start_matches('+'), self.precision).ok_or_else(|| format!("bad time '{t}'"))?;
        Ok(if t.starts_with('+') { self.sim.now() + v } else { v })
    }

    fn run_watched(&mut self, until: Option<u64>, report: &mut String) -> RunState {
        let mut first = true;
        loop {
            let st = if first {
                first = false;
                self.sim.advance(Some(self.sim.now()))
            } else {
                match self.sim.next_event_time() {
                    Some(t) if until.is_none_or(|u| t <= u) => self.sim.advance(Some(t)),
                    _ => return self.sim.advance(until),
                }
            };
            let mut hit = false;
            for (n, sig, radix, last) in self.watches.iter_mut() {
                let v = self.sim.format_signal(*sig, *radix);
                if v != *last {
                    report.push_str(&format!("{}: {n} = {v}\n", self.sim.now()));
                    *last = v;
                }
            }
            for (n, sig, value, last) in self.breaks.iter_mut() {
                let now = self.sim.format_signal(*sig, 'b');
                let fire = match value {
                    None => now != *last,
                    Some(p) => now != *last && self.sim.signal_equals(*sig, p),
                };
                *last = now;
                if fire {
                    report.push_str(&format!("break: {n} = {} at time {}\n", self.sim.format_signal(*sig, 'h'), self.sim.now()));
                    hit = true;
                }
            }
            if hit {
                return RunState::Paused;
            }
            if st == RunState::Finished {
                return st;
            }
        }
    }

    /// A signal by full name or unique suffix (after a '.').
    pub fn find(&self, name: &str) -> Result<(String, u32), String> {
        let names = self.sim.signal_names();
        if let Some((n, s)) = names.iter().find(|(n, _)| n == name) {
            return Ok((n.clone(), *s));
        }
        let suffix = format!(".{name}");
        let mut m: Vec<&(String, u32)> = names.iter().filter(|(n, _)| n.ends_with(&suffix)).collect();
        // names of one signal (a port and the net it is connected to): the shortest
        m.sort_by_key(|(n, s)| (*s, n.len()));
        m.dedup_by_key(|(_, s)| *s);
        match m.len() {
            1 => Ok((m[0].0.clone(), m[0].1)),
            0 => Err(format!("no signal '{name}'")),
            k => Err(format!("'{name}' is ambiguous ({k} signals), e.g. {} and {}", m[0].0, m[1].0)),
        }
    }

    /// `name = value` (radix b, o, d or h).
    pub fn print(&self, name: &str, radix: char) -> Result<String, String> {
        let (full, sig) = self.find(name)?;
        let (w, depth) = self.sim.signal_shape(sig);
        if depth > 0 {
            return Err(format!("{full} is an array ({depth} words)"));
        }
        Ok(format!("{full} = {w}'{radix}{}", self.sim.format_signal(sig, radix)))
    }

    /// Signal names matching a regular expression (all for ""), at most `limit`.
    pub fn list(&self, pattern: &str, limit: usize) -> Result<String, String> {
        let re = regex::Regex::new(pattern).map_err(|e| format!("bad pattern: {e}"))?;
        let names: Vec<&String> = self.sim.signal_names().iter().filter(|(n, _)| re.is_match(n)).map(|(n, _)| n).collect();
        let mut out: Vec<String> = names.iter().take(limit).map(|n| n.to_string()).collect();
        if names.len() > limit {
            out.push(format!("... {} more", names.len() - limit));
        }
        if names.is_empty() {
            out.push("(no signals match)".into());
        }
        Ok(out.join("\n"))
    }

    fn scalar(&self, name: &str) -> Result<(String, u32, u32), String> {
        let (full, sig) = self.find(name)?;
        let (w, depth) = self.sim.signal_shape(sig);
        if depth > 0 {
            return Err(format!("{full} is an array"));
        }
        Ok((full, sig, w))
    }

    /// Assign (`force` false) or force a value given as a Verilog number.
    pub fn set(&mut self, name: &str, value: &str, force: bool) -> Result<String, String> {
        self.ensure_started();
        let (full, sig, w) = self.scalar(name)?;
        let planes = parse_value(value, w).ok_or_else(|| format!("bad value '{value}'"))?;
        if force {
            self.sim.force(sig, &planes);
            Ok(format!("{full} forced to {}", self.sim.format_signal(sig, 'h')))
        } else {
            self.sim.deposit(sig, &planes);
            Ok(format!("{full} = {} (processes it wakes run on the next run/step)", self.sim.format_signal(sig, 'h')))
        }
    }

    pub fn release(&mut self, name: &str) -> Result<String, String> {
        self.ensure_started();
        let (full, sig, _) = self.scalar(name)?;
        self.sim.release(sig);
        Ok(format!("{full} released"))
    }

    pub fn watch(&mut self, name: &str, radix: char) -> Result<String, String> {
        let (full, sig, _) = self.scalar(name)?;
        self.watches.retain(|w| w.1 != sig);
        let v = self.sim.format_signal(sig, radix);
        self.watches.push((full.clone(), sig, radix, v.clone()));
        Ok(format!("watching {full} = {v}"))
    }

    pub fn unwatch(&mut self, name: &str) -> Result<String, String> {
        let (full, sig, _) = self.scalar(name)?;
        self.watches.retain(|w| w.1 != sig);
        Ok(format!("{full} no longer watched"))
    }

    pub fn add_break(&mut self, name: &str, value: Option<&str>) -> Result<String, String> {
        let (full, sig, w) = self.scalar(name)?;
        let v = match value {
            Some(v) => Some(parse_value(v, w).ok_or_else(|| format!("bad value '{v}'"))?),
            None => None,
        };
        let now = self.sim.format_signal(sig, 'b');
        let what = match value {
            Some(v) => format!("when {full} becomes {v}"),
            None => format!("when {full} changes"),
        };
        self.breaks.push((full, sig, v, now));
        Ok(format!("break {what}"))
    }

    pub fn clear_breaks(&mut self) -> String {
        let n = self.breaks.len();
        self.breaks.clear();
        format!("{n} breakpoints removed")
    }

    pub fn info(&self) -> String {
        let mut out = Vec::new();
        for (n, _, r, v) in &self.watches {
            out.push(format!("watch {n} ({r}) = {v}"));
        }
        for (n, _, v, _) in &self.breaks {
            out.push(match v {
                Some(_) => format!("break {n} == value"),
                None => format!("break {n} on change"),
            });
        }
        if out.is_empty() {
            out.push("no watches or breakpoints".into());
        }
        out.join("\n")
    }

    /// Start a VCD of the signals matching any of `patterns` (all without one).
    pub fn trace(&mut self, file: &str, patterns: &[&str]) -> Result<String, String> {
        let set = regex::RegexSet::new(patterns).map_err(|e| format!("bad pattern: {e}"))?;
        let all = patterns.is_empty();
        self.sim.trace_start(file, &|n| all || set.is_match(n)).map_err(|e| format!("cannot write {file}: {e}"))?;
        Ok(format!("tracing {} signals to {file} from time {}", self.sim.traced_signals(), self.sim.now()))
    }

    pub fn trace_enable(&mut self, on: bool) -> String {
        self.sim.trace_enable(on);
        format!("tracing {}", if on { "resumed" } else { "paused" })
    }

    /// Run the final blocks and flush the outputs.
    pub fn finish(&mut self) {
        self.ensure_started();
        self.sim.finish();
    }
}

/// A time in ticks (10^precision s), or with a unit (s, ms, us, ns, ps, fs).
pub fn parse_time(t: &str, precision: i32) -> Option<u64> {
    let units = [("fs", -15), ("ps", -12), ("ns", -9), ("us", -6), ("ms", -3), ("s", 0)];
    for (u, e) in units {
        if let Some(num) = t.strip_suffix(u) {
            let v: f64 = num.parse().ok()?;
            return Some((v * 10f64.powi(e - precision)).round() as u64);
        }
    }
    t.parse().ok()
}

/// A Verilog number as value and unknown planes of `w` bits.
pub fn parse_value(s: &str, w: u32) -> Option<Vec<u64>> {
    let n = w.div_ceil(64) as usize;
    let mut out = vec![0u64; 2 * n];
    let s = s.replace('_', "");
    let (base, digits) = match s.find('\'') {
        Some(i) => {
            let rest = &s[i + 1..];
            let rest = rest.strip_prefix(['s', 'S']).unwrap_or(rest);
            let b = rest.chars().next()?;
            match b.to_ascii_lowercase() {
                'b' => (1, rest[1..].to_string()),
                'o' => (3, rest[1..].to_string()),
                'h' => (4, rest[1..].to_string()),
                'd' => (0, rest[1..].to_string()),
                // '0 '1 'x 'z: fill
                _ => (5, rest.to_string()),
            }
        }
        None => (0, s.clone()),
    };
    let set = |out: &mut Vec<u64>, bit: u32, a: u64, b: u64| {
        if bit < w {
            let (k, sh) = ((bit / 64) as usize, bit % 64);
            out[k] |= a << sh;
            out[n + k] |= b << sh;
        }
    };
    let bitval = |c: char| -> Option<(u64, u64)> {
        match c.to_ascii_lowercase() {
            'x' => Some((1, 1)),
            'z' | '?' => Some((0, 1)),
            '0' => Some((0, 0)),
            '1' => Some((1, 0)),
            _ => None,
        }
    };
    match base {
        5 => {
            let (a, b) = bitval(digits.chars().next()?)?;
            for bit in 0..w {
                set(&mut out, bit, a, b);
            }
        }
        0 => {
            if let Some(c) = digits.chars().next().filter(|c| matches!(c.to_ascii_lowercase(), 'x' | 'z' | '?')) {
                let (a, b) = bitval(c)?;
                for bit in 0..w {
                    set(&mut out, bit, a, b);
                }
            } else {
                let (neg, d) = match digits.strip_prefix('-') {
                    Some(d) => (true, d),
                    None => (false, digits.as_str()),
                };
                let v: u128 = d.parse().ok()?;
                let v = if neg { (v as i128).wrapping_neg() as u128 } else { v };
                for bit in 0..w.min(128) {
                    set(&mut out, bit, ((v >> bit) & 1) as u64, 0);
                }
                if neg {
                    for bit in 128..w {
                        set(&mut out, bit, 1, 0);
                    }
                }
            }
        }
        bits => {
            let mut pos = 0u32;
            let mut last = (0, 0);
            for c in digits.chars().rev() {
                let (a, b) = match c.to_ascii_lowercase() {
                    'x' => (u64::MAX, u64::MAX),
                    'z' | '?' => (0, u64::MAX),
                    c => (c.to_digit(1 << bits)? as u64, 0),
                };
                for k in 0..bits {
                    set(&mut out, pos + k, (a >> k) & 1, (b >> k) & 1);
                }
                last = ((a >> (bits - 1)) & 1, (b >> (bits - 1)) & 1);
                pos += bits;
            }
            // an x or z in the leftmost digit extends to the full width
            if last.1 == 1 {
                for bit in pos..w {
                    set(&mut out, bit, last.0, 1);
                }
            }
        }
    }
    Some(out)
}
