//! MCP server (`--mcp`): the session operations as tools for an AI agent, JSON-RPC 2.0 over
//! stdio (one message per line). The design's own output ($display & co.) is captured and
//! returned with the tool results; stdout carries only the protocol.

use crate::session::{self, LoadOptions, Session};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::rc::Rc;

/// Simulation output collected between tool calls.
#[derive(Clone, Default)]
struct Captured(Rc<RefCell<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// At most this many lines of simulation output per tool result (first and last halves).
const MAX_LINES: usize = 200;

fn tools() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
    let arr = |d: &str| json!({"type": "array", "items": {"type": "string"}, "description": d});
    let tool = |name: &str, desc: &str, props: Value, req: &[&str]| {
        json!({"name": name, "description": desc,
               "inputSchema": {"type": "object", "properties": props, "required": req}})
    };
    json!([
        tool("load_design", "Parse and elaborate Verilog/SystemVerilog files and prepare the simulation at time 0 (replaces a loaded design). Reports warnings, size and compile times.",
             json!({"files": arr("source files, in order"), "defines": arr("macros: NAME or NAME=VALUE"),
                    "include_dirs": arr("`include search directories"), "top": arr("top-level modules (default: all uninstantiated)"),
                    "plusargs": arr("plusargs without the leading + (name=value)"),
                    "verilog2005": {"type": "boolean", "description": "Verilog-2005 rules where SystemVerilog differs"},
                    "specify": {"type": "boolean", "description": "apply specify path delays"}}), &["files"]),
        tool("lint", "Check files without simulating: incomplete sensitivity lists (simulation/synthesis mismatches) and names that are SystemVerilog keywords.",
             json!({"files": arr("source files"), "defines": arr("macros: NAME or NAME=VALUE"), "include_dirs": arr("include directories")}), &["files"]),
        tool("run", "Run the simulation to a time (`100ns`, `5000` ticks, `+20us` relative) or, without `until`, until nothing is scheduled or $finish. Reports watched changes, breakpoint hits, the stop reason and the design's output.",
             json!({"until": s("time to stop at")}), &[]),
        tool("step", "Run the current and the next time step.", json!({}), &[]),
        tool("status", "Current time, $finish state, watches, breakpoints, tracing.", json!({}), &[]),
        tool("get_values", "Current values of signals (hierarchical names or unique suffixes).",
             json!({"signals": arr("signal names"), "radix": s("b, o, d or h (default h)")}), &["signals"]),
        tool("list_signals", "Signal names matching a regular expression.",
             json!({"pattern": s("regular expression (default: all)"), "limit": {"type": "integer", "description": "maximum names (default 200)"}}), &[]),
        tool("set_value", "Assign a value to a signal once (a Verilog number: 5, 8'hff, 4'b10x1, 'z); processes it wakes run on the next run/step.",
             json!({"signal": s("signal name"), "value": s("Verilog number")}), &["signal", "value"]),
        tool("force", "Force a signal to a value until it is released.",
             json!({"signal": s("signal name"), "value": s("Verilog number")}), &["signal", "value"]),
        tool("release", "Release a forced signal (a net takes its drivers' value again).",
             json!({"signal": s("signal name")}), &["signal"]),
        tool("watch", "Report every change of a signal during run/step.",
             json!({"signal": s("signal name"), "radix": s("b, o, d or h")}), &["signal"]),
        tool("unwatch", "Stop watching a signal.", json!({"signal": s("signal name")}), &["signal"]),
        tool("add_breakpoint", "Stop run/step when a signal changes, or when it becomes `value`.",
             json!({"signal": s("signal name"), "value": s("Verilog number (optional)")}), &["signal"]),
        tool("clear_breakpoints", "Remove all breakpoints.", json!({}), &[]),
        tool("trace", "Write a VCD file of the signals matching the patterns (all without patterns) from now on.",
             json!({"file": s("VCD file to write"), "patterns": arr("regular expressions on hierarchical names")}), &["file"]),
        tool("trace_control", "Pause (on=false) or resume (on=true) VCD tracing.",
             json!({"on": {"type": "boolean"}}), &["on"]),
        tool("finish", "Run the final blocks and end the simulation (load_design starts a new one).", json!({}), &[]),
    ])
}

struct Server {
    ses: Option<Session>,
    out: Captured,
}

impl Server {
    fn ses(&mut self) -> Result<&mut Session, String> {
        self.ses.as_mut().ok_or_else(|| "no design loaded: call load_design first".to_string())
    }

    /// The captured simulation output since the last call, shortened to MAX_LINES.
    fn take_output(&mut self) -> String {
        if let Some(s) = &mut self.ses {
            s.sim.flush();
        }
        let bytes = std::mem::take(&mut *self.out.0.borrow_mut());
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() <= MAX_LINES {
            return text;
        }
        let h = MAX_LINES / 2;
        format!(
            "{}\n... {} lines omitted ...\n{}\n",
            lines[..h].join("\n"),
            lines.len() - 2 * h,
            lines[lines.len() - h..].join("\n")
        )
    }

    fn call(&mut self, name: &str, a: &Value) -> Result<String, String> {
        let st = |k: &str| a.get(k).and_then(Value::as_str);
        let list = |k: &str| -> Vec<String> {
            a.get(k).and_then(Value::as_array).map(|v| v.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
        };
        let need = |k: &str| st(k).ok_or_else(|| format!("missing argument '{k}'"));
        let radix = st("radix").and_then(|r| r.chars().next()).unwrap_or('h');
        let opts = || LoadOptions {
            files: list("files").into_iter().map(PathBuf::from).collect(),
            defines: list("defines")
                .into_iter()
                .map(|d| match d.split_once('=') {
                    Some((k, v)) => (k.to_string(), v.to_string()),
                    None => (d, "1".to_string()),
                })
                .collect(),
            incdirs: list("include_dirs").into_iter().map(PathBuf::from).collect(),
            tops: list("top"),
            plusargs: list("plusargs").into_iter().map(|p| p.trim_start_matches('+').to_string()).collect(),
            dialect: ferrooxide_compile::Dialect {
                v2005: a.get("verilog2005").and_then(Value::as_bool).unwrap_or(false),
                specify: a.get("specify").and_then(Value::as_bool).unwrap_or(false),
            },
            jit: true,
            fuse: true,
        };
        match name {
            "load_design" => {
                if let Some(mut old) = self.ses.take() {
                    old.finish();
                }
                let mut ses = session::load(&opts())?;
                ses.sim.set_output(Box::new(self.out.clone()));
                let r = ses.load_report.clone();
                self.ses = Some(ses);
                Ok(r)
            }
            "lint" => session::lint(&opts()),
            "run" => {
                let until = match st("until") {
                    Some(t) => Some(self.ses()?.parse_until(t)?),
                    None => None,
                };
                Ok(self.ses()?.run(until))
            }
            "step" => Ok(self.ses()?.step()),
            "status" => Ok(self.ses()?.status()),
            "get_values" => {
                let ses = self.ses()?;
                let v: Vec<String> =
                    list("signals").iter().map(|n| ses.print(n, radix).unwrap_or_else(|e| e)).collect();
                Ok(v.join("\n"))
            }
            "list_signals" => {
                let limit = a.get("limit").and_then(Value::as_u64).unwrap_or(200) as usize;
                self.ses()?.list(st("pattern").unwrap_or(""), limit)
            }
            "set_value" | "force" => {
                let (n, v) = (need("signal")?, need("value")?);
                self.ses()?.set(n, v, name == "force")
            }
            "release" => self.ses()?.release(need("signal")?),
            "watch" => self.ses()?.watch(need("signal")?, radix),
            "unwatch" => self.ses()?.unwatch(need("signal")?),
            "add_breakpoint" => {
                let n = need("signal")?;
                self.ses()?.add_break(n, st("value"))
            }
            "clear_breakpoints" => Ok(self.ses()?.clear_breaks()),
            "trace" => {
                let pats = list("patterns");
                let refs: Vec<&str> = pats.iter().map(String::as_str).collect();
                self.ses()?.trace(need("file")?, &refs)
            }
            "trace_control" => {
                let on = a.get("on").and_then(Value::as_bool).ok_or("missing argument 'on'")?;
                Ok(self.ses()?.trace_enable(on))
            }
            "finish" => {
                let mut ses = self.ses.take().ok_or("no design loaded")?;
                ses.finish();
                Ok(format!("finished at time {}", ses.time()))
            }
            _ => Err(format!("unknown tool '{name}'")),
        }
    }

    fn handle(&mut self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "ferrooxide", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Verilog simulator. load_design, then run/step and inspect with get_values, watch, add_breakpoint; force/set_value/release change signals; trace writes a VCD. Times: ticks of the design's precision or with a unit (100ns)."
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let r = self.call(name, &args);
                let output = self.take_output();
                let (mut text, err) = match r {
                    Ok(t) => (t, false),
                    Err(e) => (e, true),
                };
                if !output.is_empty() {
                    text.push_str("\n--- simulation output ---\n");
                    text.push_str(&output);
                }
                Ok(json!({"content": [{"type": "text", "text": text}], "isError": err}))
            }
            m if m.starts_with("notifications/") => return None,
            _ => Err(json!({"code": -32601, "message": format!("method not found: {method}")})),
        };
        let id = id?; // notifications get no answer
        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
        })
    }
}

/// Serve MCP on stdin/stdout. `initial`: a design to load at start (from the command line).
pub fn serve(initial: Option<LoadOptions>) {
    let mut srv = Server { ses: None, out: Captured::default() };
    if let Some(o) = initial {
        match session::load(&o) {
            Ok(mut s) => {
                s.sim.set_output(Box::new(srv.out.clone()));
                eprintln!("ferrooxide mcp: {}", s.load_report);
                srv.ses = Some(s);
            }
            Err(e) => eprintln!("ferrooxide mcp: {e}"),
        }
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => srv.handle(&msg),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}})),
        };
        if let Some(r) = reply {
            let _ = writeln!(stdout, "{r}");
            let _ = stdout.flush();
        }
    }
    if let Some(mut s) = srv.ses.take() {
        s.finish();
    }
}
