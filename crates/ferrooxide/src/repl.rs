//! Interactive mode (`-i`): commands on stdin to run, inspect, force and release.

use ferrooxide_rt::{RunState, Sim};
use std::io::{BufRead, IsTerminal, Write};

const HELP: &str = "commands:
  run [T | +T]        run to time T (or T more); without T until nothing is scheduled
  step                run the current and the next time step
  time                show the current time
  print NAME [b|o|d|h] (p) show a signal's value (default hex)
  list [TEXT]         (ls) list signals whose name contains TEXT
  set NAME VALUE      assign a value (processes it wakes run on the next run/step)
  force NAME VALUE    force a signal until release
  release NAME        release a forced signal
  watch NAME [b|o|d|h] print NAME whenever it changes during run/step (unwatch NAME)
  break NAME [VALUE]  stop when NAME changes (or becomes VALUE); delete removes all
  info                list watches and breakpoints
  help                (h) this text
  quit                (q) run final blocks and exit
NAME is a hierarchical name or a unique suffix of one (`top.u1.q`, `u1.q`, `q`).
VALUE is a Verilog number: 5, 8'hff, 4'b10x1, 'z. T is in simulation ticks or has a unit:
10ns, 2.5us.";

pub fn run(sim: &mut Sim, precision: i32) {
    let stdin = std::io::stdin();
    let tty = stdin.is_terminal();
    let mut finished = false;
    // watched signals: (name, signal, radix, last value); breakpoints: (name, signal, value)
    let mut watches: Vec<(String, u32, char, String)> = Vec::new();
    let mut breaks: Vec<(String, u32, Option<Vec<u64>>, String)> = Vec::new();
    sim.start();
    let mut lines = stdin.lock().lines();
    loop {
        if tty {
            print!("fx> ");
            let _ = std::io::stdout().flush();
        }
        let Some(Ok(line)) = lines.next() else { break };
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(&cmd) = words.first() else { continue };
        let arg = |k: usize| words.get(k).copied();
        match cmd {
            "help" | "h" | "?" => println!("{HELP}"),
            "quit" | "q" | "exit" => break,
            "time" => println!("time {}", sim.time),
            "run" | "r" | "step" | "s" | "continue" | "c" => {
                if finished {
                    println!("the simulation has finished ($finish)");
                    continue;
                }
                let until = if cmd.starts_with('s') {
                    match sim.next_event_time() {
                        Some(t) => Some(t),
                        None => Some(sim.time),
                    }
                } else {
                    match arg(1) {
                        None => None,
                        Some(t) => match parse_time(t.trim_start_matches('+'), precision) {
                            Some(v) if t.starts_with('+') => Some(sim.time + v),
                            Some(v) => Some(v),
                            None => {
                                println!("bad time '{t}'");
                                continue;
                            }
                        },
                    }
                };
                let st = if watches.is_empty() && breaks.is_empty() {
                    sim.advance(until)
                } else {
                    run_watched(sim, until, &mut watches, &mut breaks)
                };
                sim.flush();
                match st {
                    RunState::Paused => println!("stopped at time {}", sim.time),
                    RunState::Idle => println!("time {}: nothing more is scheduled", sim.time),
                    RunState::Finished => {
                        finished = true;
                        println!("time {}: $finish", sim.time)
                    }
                }
            }
            "print" | "p" => {
                let Some(name) = arg(1) else {
                    println!("print NAME [b|o|d|h]");
                    continue;
                };
                let radix = arg(2).and_then(|r| r.chars().next()).unwrap_or('h');
                if let Some((full, sig)) = find(sim, name) {
                    let (w, depth) = sim.signal_shape(sig);
                    if depth > 0 {
                        println!("{full} is an array ({depth} words)");
                    } else {
                        println!("{full} = {w}'{radix}{}", sim.format_signal(sig, radix));
                    }
                }
            }
            "list" | "ls" => {
                let pat = arg(1).unwrap_or("");
                let names: Vec<String> =
                    sim.signal_names().iter().filter(|(n, _)| n.contains(pat)).map(|(n, _)| n.clone()).collect();
                for n in names.iter().take(500) {
                    println!("{n}");
                }
                if names.len() > 500 {
                    println!("... {} more", names.len() - 500);
                }
            }
            "set" | "deposit" | "force" => {
                let (Some(name), Some(val)) = (arg(1), arg(2)) else {
                    println!("{cmd} NAME VALUE");
                    continue;
                };
                let Some((full, sig)) = find(sim, name) else { continue };
                let (w, depth) = sim.signal_shape(sig);
                if depth > 0 {
                    println!("{full} is an array");
                    continue;
                }
                let Some(planes) = parse_value(val, w) else {
                    println!("bad value '{val}'");
                    continue;
                };
                if cmd == "force" {
                    sim.force(sig, &planes);
                } else {
                    sim.deposit(sig, &planes);
                }
            }
            "watch" | "unwatch" => {
                let Some(name) = arg(1) else {
                    println!("{cmd} NAME");
                    continue;
                };
                let Some((full, sig)) = find(sim, name) else { continue };
                watches.retain(|w| w.1 != sig);
                if cmd == "watch" {
                    let radix = arg(2).and_then(|r| r.chars().next()).unwrap_or('h');
                    let v = sim.format_signal(sig, radix);
                    println!("{full} = {v}");
                    watches.push((full, sig, radix, v));
                }
            }
            "break" | "b" => {
                let Some(name) = arg(1) else {
                    println!("break NAME [VALUE]");
                    continue;
                };
                let Some((full, sig)) = find(sim, name) else { continue };
                let (w, _) = sim.signal_shape(sig);
                let value = match arg(2) {
                    Some(v) => match parse_value(v, w) {
                        Some(p) => Some(p),
                        None => {
                            println!("bad value '{v}'");
                            continue;
                        }
                    },
                    None => None,
                };
                let now = sim.format_signal(sig, 'b');
                breaks.push((full, sig, value, now));
            }
            "delete" => breaks.clear(),
            "info" => {
                for (n, _, r, v) in &watches {
                    println!("watch {n} ({r}) = {v}");
                }
                for (n, _, v, _) in &breaks {
                    match v {
                        Some(_) => println!("break {n} == value"),
                        None => println!("break {n} on change"),
                    }
                }
            }
            "release" => {
                let Some(name) = arg(1) else {
                    println!("release NAME");
                    continue;
                };
                if let Some((_, sig)) = find(sim, name) {
                    sim.release(sig);
                }
            }
            _ => println!("unknown command '{cmd}' (help lists the commands)"),
        }
    }
    sim.finish();
}

/// Run time step by time step up to `until`, printing watched signals that change and stopping
/// at breakpoints.
fn run_watched(
    sim: &mut Sim,
    until: Option<u64>,
    watches: &mut [(String, u32, char, String)],
    breaks: &mut [(String, u32, Option<Vec<u64>>, String)],
) -> RunState {
    let mut first = true;
    loop {
        let st = if first {
            // the current time step (it may still have work, e.g. at time 0)
            first = false;
            sim.advance(Some(sim.time))
        } else {
            match sim.next_event_time() {
                Some(t) if until.is_none_or(|u| t <= u) => sim.advance(Some(t)),
                // nothing more before `until`
                _ => return sim.advance(until),
            }
        };
        let mut hit = false;
        for (n, sig, radix, last) in watches.iter_mut() {
            let v = sim.format_signal(*sig, *radix);
            if v != *last {
                sim.flush();
                println!("{}: {n} = {v}", sim.time);
                *last = v;
            }
        }
        for (n, sig, value, last) in breaks.iter_mut() {
            let now = sim.format_signal(*sig, 'b');
            let fire = match value {
                None => now != *last,
                Some(p) => now != *last && sim.signal_equals(*sig, p),
            };
            *last = now;
            if fire {
                sim.flush();
                println!("break: {n} = {} at time {}", sim.format_signal(*sig, 'h'), sim.time);
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
fn find(sim: &Sim, name: &str) -> Option<(String, u32)> {
    let names = sim.signal_names();
    if let Some((n, s)) = names.iter().find(|(n, _)| n == name) {
        return Some((n.clone(), *s));
    }
    let suffix = format!(".{name}");
    let mut m: Vec<&(String, u32)> = names.iter().filter(|(n, _)| n.ends_with(&suffix)).collect();
    // names of one signal (a port and the net it is connected to): the shortest
    m.sort_by_key(|(n, s)| (*s, n.len()));
    m.dedup_by_key(|(_, s)| *s);
    match m.len() {
        1 => Some((m[0].0.clone(), m[0].1)),
        0 => {
            println!("no signal '{name}'");
            None
        }
        k => {
            println!("'{name}' is ambiguous ({k} signals), e.g. {} and {}", m[0].0, m[1].0);
            None
        }
    }
}

/// A time in ticks (10^precision s), or with a unit (s, ms, us, ns, ps, fs).
fn parse_time(t: &str, precision: i32) -> Option<u64> {
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
fn parse_value(s: &str, w: u32) -> Option<Vec<u64>> {
    let n = w.div_ceil(64) as usize;
    let mut out = vec![0u64; 2 * n];
    let s = s.replace('_', "");
    let (base, digits, fill_ext) = match s.find('\'') {
        Some(i) => {
            let rest = &s[i + 1..];
            let rest = rest.strip_prefix(['s', 'S']).unwrap_or(rest);
            let b = rest.chars().next()?;
            let (bits, d) = match b.to_ascii_lowercase() {
                'b' => (1, &rest[1..]),
                'o' => (3, &rest[1..]),
                'h' => (4, &rest[1..]),
                'd' => (0, &rest[1..]),
                // '0 '1 'x 'z: fill
                _ => (5, rest),
            };
            (bits, d.to_string(), i == 0)
        }
        None => (0, s.clone(), false),
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
    let _ = fill_ext;
    Some(out)
}
