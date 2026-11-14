//! Interactive mode (`-i`): commands on stdin to run, inspect, force and release.

use crate::session::Session;
use std::io::{BufRead, IsTerminal, Write};

const HELP: &str = "commands:
  run [T | +T]        run to time T (or T more); without T until nothing is scheduled
  step                run the current and the next time step
  time                show the current time
  print NAME [b|o|d|h] (p) show a signal's value (default hex)
  list [REGEX]        (ls) list signals whose name matches
  set NAME VALUE      assign a value (processes it wakes run on the next run/step)
  force NAME VALUE    force a signal until release
  release NAME        release a forced signal
  watch NAME [b|o|d|h] print NAME whenever it changes during run/step (unwatch NAME)
  break NAME [VALUE]  stop when NAME changes (or becomes VALUE); delete removes all
  info                list watches and breakpoints
  trace FILE [RE...]  write a VCD of the signals whose name matches a regular expression
                      (all without one), from now on; trace off / trace on pause and resume
  help                (h) this text
  quit                (q) run final blocks and exit
NAME is a hierarchical name or a unique suffix of one (`top.u1.q`, `u1.q`, `q`).
VALUE is a Verilog number: 5, 8'hff, 4'b10x1, 'z. T is in simulation ticks or has a unit:
10ns, 2.5us.";

/// Print the prompt when stdin is a terminal.
pub fn prompt() {
    if std::io::stdin().is_terminal() {
        print!("fx> ");
        let _ = std::io::stdout().flush();
    }
}

pub fn run(ses: &mut Session) {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        prompt();
        let Some(Ok(line)) = lines.next() else { break };
        match execute(ses, &line) {
            Some(out) if out.is_empty() => {}
            Some(out) => println!("{out}"),
            None => break,
        }
    }
    ses.finish();
}

/// Run one command line; its report (empty for a blank line), or None for quit.
pub fn execute(ses: &mut Session, line: &str) -> Option<String> {
    {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(&cmd) = words.first() else { return Some(String::new()) };
        let arg = |k: usize| words.get(k).copied();
        let radix = |k: usize| arg(k).and_then(|r| r.chars().next()).unwrap_or('h');
        let need = |k: usize, usage: &str| -> Result<String, String> { arg(k).map(str::to_string).ok_or_else(|| usage.to_string()) };
        let r: Result<String, String> = match cmd {
            "help" | "h" | "?" => Ok(HELP.into()),
            "quit" | "q" | "exit" => return None,
            "time" => Ok(format!("time {}", ses.time())),
            "run" | "r" | "continue" | "c" => match arg(1) {
                None => Ok(ses.run(None)),
                Some(t) => ses.parse_until(t).map(|u| ses.run(Some(u))),
            },
            "step" | "s" => Ok(ses.step()),
            "print" | "p" => need(1, "print NAME [b|o|d|h]").and_then(|n| ses.print(&n, radix(2))),
            "list" | "ls" => ses.list(arg(1).unwrap_or(""), 500),
            "set" | "deposit" | "force" => match (arg(1), arg(2)) {
                (Some(n), Some(v)) => ses.set(n, v, cmd == "force"),
                _ => Err(format!("{cmd} NAME VALUE")),
            },
            "release" => need(1, "release NAME").and_then(|n| ses.release(&n)),
            "watch" => need(1, "watch NAME [b|o|d|h]").and_then(|n| ses.watch(&n, radix(2))),
            "unwatch" => need(1, "unwatch NAME").and_then(|n| ses.unwatch(&n)),
            "break" | "b" => need(1, "break NAME [VALUE]").and_then(|n| ses.add_break(&n, arg(2))),
            "delete" => Ok(ses.clear_breaks()),
            "info" => Ok(ses.info()),
            "trace" => match arg(1) {
                None => Ok(ses.status()),
                Some("off") => Ok(ses.trace_enable(false)),
                Some("on") => Ok(ses.trace_enable(true)),
                Some(file) => ses.trace(file, &words[2..]),
            },
            _ => Err(format!("unknown command '{cmd}' (help lists the commands)")),
        };
        Some(match r {
            Ok(s) => s,
            Err(e) => e,
        })
    }
}
