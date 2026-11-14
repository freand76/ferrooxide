//! CXXRTL Debug Server Protocol server (`--cdsp [HOST:]PORT`): waveform viewers such as Surfer
//! (`surfer cxxrtl+tcp://HOST:PORT`) attach to the running simulation, run and pause it and query
//! the value history of any signal. Spec: https://cxxrtl.org/protocol.html (protocol version 0).
//!
//! With `--surfer`, ferrooxide also starts Surfer and drives it over Surfer's Waveform Control
//! Protocol (WCP, same framing): Surfer connects back (`--wcp-initiate`), and the initial
//! signals (`--trace` patterns, else the top modules') are added to its view; without a console the
//! simulation runs at once (pausing after a while if it does not end); `$finish`
//! gets a marker. Surfer reports no `waveforms_loaded` for a CXXRTL source, so `add_items` is
//! retried while Surfer answers that no waveform is loaded.
//!
//! With `-i`, the interactive console runs next to the server (stdin): its run/step commands
//! (and breakpoints) move the simulation; the viewer then gets `simulation_paused` and, with
//! `--surfer`, the cursor moves to the new time.
//!
//! Messages are JSON objects terminated by NUL. One client at a time (a new connection replaces
//! the old one); the simulation keeps its state across connections. Values are sent as
//! `base64(u32)`, which is two-state: x is sent as 1, z as 0. Reals are 64-bit items holding the
//! IEEE double. Arrays are not listed yet (Surfer designates every item as a plain node).

use crate::session::Session;
use ferrooxide_rt::{RunState, Sim};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex, Once};
use std::time::{Duration, Instant};

enum In {
    Connected(u32, TcpStream),
    Msg(u32, String),
    Closed(u32),
    WcpConnected(TcpStream),
    WcpMsg(String),
    WcpClosed,
    ViewerExited,
    Console(String),
    ConsoleEof,
}

/// Start a viewer (Surfer) that attaches to the server and is driven over WCP.
pub struct Viewer {
    /// the Surfer executable
    pub bin: String,
    /// regular expressions selecting the signals shown first (empty: the top modules' signals)
    pub traces: Vec<String>,
}

/// The WCP side: the connection to the viewer and what is still to be sent.
#[derive(Default)]
struct Wcp {
    out: Option<TcpStream>,
    /// the viewer's greeting arrived
    ready: bool,
    /// signals (dotted names) to add once a waveform is loaded in the viewer
    pending: Option<Vec<String>>,
    /// the view was zoomed to fit the first values
    zoomed: bool,
    /// when to try `add_items` again
    retry_at: Option<Instant>,
    /// commands sent and not answered yet (WCP answers in order)
    inflight: VecDeque<&'static str>,
}

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Paused,
    Running(Option<u64>),
    Finished,
}

struct Item {
    sig: u32,
    width: u32,
    lsb: i64,
}

struct Server {
    ses: Session,
    status: Status,
    /// femtoseconds per tick
    fs_per_tick: u128,
    /// scope id ("a b") -> nothing; the root scope is ""
    scopes: BTreeSet<String>,
    /// item id ("a b sig") -> item
    items: BTreeMap<String, Item>,
    /// reference name -> designated items
    refs: HashMap<String, Vec<String>>,
    out: Option<(u32, TcpStream)>,
    warned_full: bool,
    wcp: Wcp,
    /// running on its own (`--surfer` without a console): pause at this wall time, so a
    /// simulation without an end still shows its waves
    auto_pause: Option<Instant>,
    /// run on its own once the viewer has referenced its signals
    auto_run: bool,
    /// time steps per `advance_steps` call while running
    chunk: u64,
}

const COMMANDS: &[&str] = &[
    "list_scopes",
    "list_items",
    "reference_items",
    "query_interval",
    "get_simulation_status",
    "run_simulation",
    "pause_simulation",
];

#[cfg(test)]
fn base64(bytes: &[u8]) -> String {
    let mut s = String::new();
    base64_into(bytes, &mut s);
    s
}

fn base64_into(bytes: &[u8], s: &mut String) {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    s.reserve(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
}

fn parent(id: &str) -> &str {
    id.rfind(' ').map_or("", |i| &id[..i])
}

impl Server {
    fn new(ses: Session) -> Server {
        let sim = &ses.sim;
        let fs_per_tick = 10u128.pow((15 + sim.precision()).clamp(0, 30) as u32);
        let mut scopes = BTreeSet::new();
        scopes.insert(String::new());
        let mut items = BTreeMap::new();
        for (name, sig) in sim.signal_names() {
            let info = sim.signal_info(*sig);
            if info.depth > 0 || name.contains(' ') {
                continue;
            }
            let id = name.replace('.', " ");
            let mut p = parent(&id);
            while !p.is_empty() {
                scopes.insert(p.to_string());
                p = parent(p);
            }
            items.insert(id, Item { sig: *sig, width: info.width, lsb: info.lsb.min(info.msb) });
        }
        Server { ses, status: Status::Paused, fs_per_tick, scopes, items, refs: HashMap::new(), out: None, warned_full: false, wcp: Wcp::default(), chunk: 64, auto_pause: None, auto_run: false }
    }

    fn time_str(&self, ticks: u64) -> String {
        let fs = ticks as u128 * self.fs_per_tick;
        format!("{}.{:015}", fs / 1_000_000_000_000_000, fs % 1_000_000_000_000_000)
    }

    /// A time point ("s.fs") in ticks (rounded down).
    fn parse_time(&self, v: &Value) -> Option<u64> {
        let (s, f) = v.as_str()?.split_once('.')?;
        let s: u128 = s.parse().ok()?;
        let digits = f.len();
        let f: u128 = f.parse().ok()?;
        // the fraction is femtoseconds when it has 15 digits; scale shorter or longer ones
        let fs = if digits <= 15 { f * 10u128.pow(15 - digits as u32) } else { f / 10u128.pow(digits as u32 - 15) };
        Some(((s * 1_000_000_000_000_000 + fs) / self.fs_per_tick).min(u64::MAX as u128) as u64)
    }

    fn send(&mut self, v: Value) {
        self.send_text(serde_json::to_string(&v).unwrap());
    }

    fn send_text(&mut self, mut text: String) {
        if let Some((_, s)) = &mut self.out {
            text.push('\0');
            if s.write_all(text.as_bytes()).is_err() {
                self.out = None;
            }
        }
    }

    fn error(&mut self, error: &str, message: String) {
        self.send(json!({"type": "error", "error": error, "message": message}));
    }

    fn scope_desc() -> Value {
        json!({"type": "module",
               "definition": {"src": null, "name": null, "attributes": {}},
               "instantiation": {"src": null, "attributes": {}}})
    }

    fn handle(&mut self, text: &str) {
        let m: Value = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(e) => return self.error("invalid_message", format!("not JSON: {e}")),
        };
        match m["type"].as_str() {
            Some("greeting") => {
                if m["version"] != json!(0) {
                    return self.error("invalid_version", format!("protocol version {} not supported", m["version"]));
                }
                self.send(json!({"type": "greeting", "version": 0, "commands": COMMANDS,
                    "events": ["simulation_paused", "simulation_finished"],
                    "features": {"item_values_encoding": ["base64(u32)"]}}));
            }
            Some("command") => self.command(&m),
            _ => self.error("invalid_message", "expected a greeting or a command".into()),
        }
    }

    fn command(&mut self, m: &Value) {
        let cmd = m["command"].as_str().unwrap_or("");
        match cmd {
            "list_scopes" => {
                let mut scopes = Map::new();
                for s in &self.scopes {
                    let keep = match m["scope"].as_str() {
                        None => true,
                        Some(p) => !s.is_empty() && parent(s) == p,
                    };
                    if keep {
                        scopes.insert(s.clone(), Self::scope_desc());
                    }
                }
                self.send(json!({"type": "response", "command": cmd, "scopes": scopes}));
            }
            "list_items" => {
                let mut items = Map::new();
                for (id, it) in &self.items {
                    if m["scope"].as_str().is_none_or(|p| parent(id) == p) {
                        items.insert(id.clone(), json!({"src": null, "type": "node", "width": it.width, "lsb_at": it.lsb,
                            "settable": false, "input": false, "output": false, "attributes": {}}));
                    }
                }
                self.send(json!({"type": "response", "command": cmd, "items": items}));
            }
            "reference_items" => {
                let Some(name) = m["reference"].as_str().filter(|r| !r.is_empty()) else {
                    return self.error("invalid_reference", "the reference name is empty".into());
                };
                if m["items"].is_null() {
                    self.refs.remove(name);
                    return self.send(json!({"type": "response", "command": cmd}));
                }
                let mut ids = Vec::new();
                for d in m["items"].as_array().into_iter().flatten() {
                    let id = d.get(0).and_then(Value::as_str).unwrap_or("");
                    if d.as_array().map_or(0, Vec::len) != 1 || !self.items.contains_key(id) {
                        return self.error("invalid_item", format!("unknown or unsupported item designation {d}"));
                    }
                    ids.push(id.to_string());
                }
                self.refs.insert(name.to_string(), ids);
                self.send(json!({"type": "response", "command": cmd}));
                if self.auto_run && self.status == Status::Paused {
                    self.auto_run = false;
                    self.status = Status::Running(None);
                    self.auto_pause = Some(Instant::now() + AUTO_RUN);
                }
            }
            "query_interval" => {
                self.query_interval(m);
                // the first values after time 0: show the whole run (later the viewer's own
                // zoom is kept)
                if self.wcp.ready && !self.wcp.zoomed && self.ses.time() > 0 {
                    self.wcp.zoomed = true;
                    self.wcp_send("zoom_to_fit", json!({"type": "command", "command": "zoom_to_fit", "viewport_idx": 0}));
                }
            }
            "get_simulation_status" => {
                let mut r = json!({"type": "response", "command": cmd, "latest_time": self.time_str(self.ses.sim.now())});
                r["status"] = json!(match self.status {
                    Status::Paused => "paused",
                    Status::Running(_) => "running",
                    Status::Finished => "finished",
                });
                if self.status == Status::Paused {
                    let next = self.ses.sim.next_event_time().unwrap_or(self.ses.sim.now());
                    r["next_sample_time"] = json!(self.time_str(next));
                }
                self.send(r);
            }
            "run_simulation" => {
                if self.status != Status::Paused {
                    return self.error("invalid_status", "the simulation is not paused".into());
                }
                let until = if m["until_time"].is_null() {
                    None
                } else {
                    match self.parse_time(&m["until_time"]) {
                        Some(t) => Some(t),
                        None => return self.error("invalid_args", format!("bad until_time {}", m["until_time"])),
                    }
                };
                self.status = Status::Running(until);
                self.send(json!({"type": "response", "command": cmd}));
            }
            "pause_simulation" => {
                self.auto_pause = None;
                if let Status::Running(_) = self.status {
                    self.status = Status::Paused;
                    self.ses.sim.flush();
                }
                let t = self.time_str(self.ses.sim.now());
                self.send(json!({"type": "response", "command": cmd, "time": t}));
            }
            _ => self.error("invalid_command", format!("unknown command '{cmd}'")),
        }
    }

    fn query_interval(&mut self, m: &Value) {
        let (Some(b), Some(e)) = (self.parse_time(&m["interval"][0]), self.parse_time(&m["interval"][1])) else {
            return self.error("invalid_args", format!("bad interval {}", m["interval"]));
        };
        let now = self.ses.sim.now();
        if b > e || e > now {
            return self.error("invalid_args", format!("interval {} is outside [0, latest time]", m["interval"]));
        }
        let ids = match m["items"].as_str() {
            None => None,
            Some(r) => match self.refs.get(r) {
                Some(ids) => Some(ids),
                None => return self.error("invalid_reference", format!("unknown reference '{r}'")),
            },
        };
        let values = ids.is_some() && !m["item_values_encoding"].is_null();
        if values && m["item_values_encoding"] != json!("base64(u32)") {
            return self.error("invalid_args", format!("unsupported encoding {}", m["item_values_encoding"]));
        }
        let sigs: Vec<(u32, u32)> =
            ids.into_iter().flatten().map(|id| (self.items[id].sig, self.items[id].width)).collect();
        let only: Vec<u32> = sigs.iter().map(|&(s, _)| s).collect();
        self.ses.sim.history_mut().unwrap().prepare(&only);
        let h = self.ses.sim.history().unwrap();
        // sample times: the interval start, then every change of a referenced item
        let mut times = vec![b];
        for &(s, _) in &sigs {
            for i in h.count_upto(s, b)..h.count_upto(s, e) {
                times.push(h.time(s, i));
            }
        }
        times.sort_unstable();
        times.dedup();
        let mut cur: Vec<Option<usize>> = sigs.iter().map(|&(s, _)| h.index_at(s, b)).collect();
        // written as text: a long interval has hundreds of thousands of samples
        let diags = m["diagnostics"].as_bool() == Some(true);
        let mut out = String::with_capacity(64 + times.len() * (48 + if values { sigs.len() * 8 } else { 0 }));
        out.push_str(r#"{"command":"query_interval","samples":["#);
        let mut bytes = Vec::new();
        let mut plane = Vec::new();
        for (n, &t) in times.iter().enumerate() {
            if n > 0 {
                out.push(',');
            }
            out.push('{');
            if diags {
                out.push_str(r#""diagnostics":[],"#);
            }
            if values {
                bytes.clear();
                for (k, &(s, w)) in sigs.iter().enumerate() {
                    let mut i = cur[k].map_or(0, |i| i + 1);
                    while i < h.count(s) && h.time(s, i) <= t {
                        i += 1;
                    }
                    cur[k] = i.checked_sub(1);
                    plane.clear();
                    plane.resize(w.div_ceil(64) as usize, 0);
                    if let Some(i) = cur[k] {
                        h.value_plane(s, i, &mut plane);
                    }
                    for j in 0..w.div_ceil(32) as usize {
                        bytes.extend_from_slice(&((plane[j / 2] >> (32 * (j % 2))) as u32).to_le_bytes());
                    }
                }
                out.push_str(r#""item_values":""#);
                base64_into(&bytes, &mut out);
                out.push_str(r#"","#);
            }
            out.push_str(r#""time":""#);
            out.push_str(&self.time_str(t));
            out.push_str(r#""}"#);
        }
        out.push_str(r#"],"type":"response"}"#);
        self.send_text(out);
    }

    /// Run a chunk of time steps of a running simulation (sized to take 10-40 ms, so a pause
    /// is handled quickly); emits the event when it stops.
    fn step(&mut self) {
        let Status::Running(until) = self.status else { return };
        let t0 = Instant::now();
        let st = self.ses.sim.advance_steps(until, self.chunk);
        let dt = t0.elapsed();
        if dt < Duration::from_millis(10) {
            self.chunk = (self.chunk * 2).min(1 << 20);
        } else if dt > Duration::from_millis(40) {
            self.chunk = (self.chunk / 2).max(1);
        }
        let auto = st == RunState::Paused && self.auto_pause.is_some_and(|t| Instant::now() >= t);
        if auto {
            self.auto_pause = None;
            eprintln!("ferrooxide: paused at time {} after running for a while; continue from the viewer", self.ses.sim.now());
        }
        if auto || (st == RunState::Paused && until.is_some_and(|u| self.ses.sim.now() >= u)) {
            self.status = Status::Paused;
            self.ses.sim.flush();
            let t = self.time_str(self.ses.sim.now());
            self.send(json!({"type": "event", "event": "simulation_paused", "time": t, "cause": "until_time"}));
            return;
        }
        if let Some(h) = self.ses.sim.history() {
            if let (Some(t), false) = (h.full_at, self.warned_full) {
                self.warned_full = true;
                eprintln!("ferrooxide: value history full at time {t} (FX_HISTORY_MB); later changes are not recorded");
            }
        }
        if st != RunState::Paused {
            self.status = Status::Finished;
            self.ses.sim.finish();
            self.ses.mark_finished();
            let t = self.time_str(self.ses.sim.now());
            self.send(json!({"type": "event", "event": "simulation_finished", "time": t}));
            self.mark_finish(if st == RunState::Finished { "$finish" } else { "end" });
        }
    }

    /// A console command (`-i`); false for quit. A command that moved the simulation is
    /// reported to the viewer, and the viewer's cursor follows it.
    fn console(&mut self, line: &str) -> bool {
        let was_running = matches!(self.status, Status::Running(_));
        if was_running {
            self.status = Status::Paused; // the console takes over
        }
        let t0 = self.ses.time();
        match crate::repl::execute(&mut self.ses, line) {
            None => return false,
            Some(out) if out.is_empty() => {}
            Some(out) => println!("{out}"),
        }
        if self.ses.is_finished() && self.status != Status::Finished {
            self.status = Status::Finished;
            let t = self.time_str(self.ses.sim.now());
            self.send(json!({"type": "event", "event": "simulation_finished", "time": t}));
            self.mark_finish("$finish");
        } else if was_running || self.ses.time() != t0 {
            let t = self.time_str(self.ses.sim.now());
            self.send(json!({"type": "event", "event": "simulation_paused", "time": t, "cause": "until_time"}));
            if self.wcp.ready && self.ses.time() != t0 {
                let fs = self.ses.time() as u128 * self.fs_per_tick;
                self.wcp_send("set_cursor", json!({"type": "command", "command": "set_cursor", "timestamp": fs as u64}));
            }
        }
        crate::repl::prompt();
        true
    }

    // ------------------------------------------------------------ WCP (viewer control)

    fn wcp_send(&mut self, kind: &'static str, v: Value) {
        if let Some(s) = &mut self.wcp.out {
            let mut b = serde_json::to_vec(&v).unwrap();
            b.push(0);
            if s.write_all(&b).is_err() {
                self.wcp.out = None;
                return;
            }
            if !kind.is_empty() {
                self.wcp.inflight.push_back(kind);
            }
        }
    }

    /// Send `add_items` when the viewer is ready and the retry time has come.
    fn wcp_poll(&mut self) {
        let due = self.wcp.retry_at.is_none_or(|t| Instant::now() >= t);
        if !self.wcp.ready || !due || self.wcp.inflight.contains(&"add_items") {
            return;
        }
        if let Some(items) = self.wcp.pending.clone() {
            self.wcp.retry_at = None;
            self.wcp_send("add_items", json!({"type": "command", "command": "add_items", "items": items, "recursive": false}));
        }
    }

    fn wcp_handle(&mut self, text: &str) {
        let Ok(m) = serde_json::from_str::<Value>(text) else { return };
        match m["type"].as_str() {
            Some("greeting") => {
                self.wcp.ready = true;
                self.wcp_poll();
            }
            Some("response") => {
                if self.wcp.inflight.pop_front() == Some("add_items") {
                    self.wcp.pending = None;
                }
            }
            Some("error") => {
                if self.wcp.inflight.pop_front() == Some("add_items") {
                    // most likely the CXXRTL source is not connected yet
                    self.wcp.retry_at = Some(Instant::now() + Duration::from_millis(250));
                }
            }
            _ => {}
        }
    }

    /// A marker at the time of $finish (or the end of the events) in the viewer.
    fn mark_finish(&mut self, name: &str) {
        if self.wcp.ready {
            let fs = self.ses.sim.now() as u128 * self.fs_per_tick;
            self.wcp_send("add_markers", json!({"type": "command", "command": "add_markers",
                "markers": [{"time": fs as u64, "name": name, "move_focus": false}]}));
        }
    }
}

/// The signals shown first: those matching `traces`, else those of the top modules (not
/// arrays; at most `MAX_SHOWN`).
const MAX_SHOWN: usize = 2000;
fn initial_items(sim: &Sim, traces: &[String]) -> Result<Vec<String>, String> {
    let set = regex::RegexSet::new(traces).map_err(|e| format!("bad --trace pattern: {e}"))?;
    let mut v: Vec<String> = sim
        .signal_names()
        .iter()
        .filter(|(n, s)| {
            let info = sim.signal_info(*s);
            info.depth == 0
                && !n.contains(' ')
                && if traces.is_empty() { n.matches('.').count() == 1 } else { set.is_match(n) }
        })
        .map(|(n, _)| n.clone())
        .collect();
    v.sort();
    v.dedup();
    if v.len() > MAX_SHOWN {
        eprintln!("ferrooxide: {} signals; the viewer shows the first {MAX_SHOWN} (select with --trace)", v.len());
        v.truncate(MAX_SHOWN);
    }
    Ok(v)
}

fn reader(mut s: TcpStream, tx: mpsc::Sender<In>, msg: impl Fn(String) -> In, closed: In) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1 << 16];
    loop {
        let n = match s.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        while let Some(i) = buf.iter().position(|&c| c == 0) {
            let text = String::from_utf8_lossy(&buf[..i]).into_owned();
            buf.drain(..=i);
            if tx.send(msg(text)).is_err() {
                return;
            }
        }
    }
    let _ = tx.send(closed);
}

const AUTO_RUN: Duration = Duration::from_secs(2);

/// Console lines go to the current server (one stdin reader for all restarts).
static CONSOLE: Mutex<Option<mpsc::Sender<In>>> = Mutex::new(None);
static CONSOLE_READER: Once = Once::new();

/// Close the connections, give the viewer a moment to exit on its own, then stop it.
fn close_viewer(srv: &mut Server, child: Option<&Arc<Mutex<std::process::Child>>>) {
    if let Some((_, s)) = srv.out.take() {
        let _ = s.shutdown(std::net::Shutdown::Both);
    }
    if let Some(s) = srv.wcp.out.take() {
        let _ = s.shutdown(std::net::Shutdown::Both);
    }
    let Some(c) = child else { return };
    for _ in 0..10 {
        if !matches!(c.lock().unwrap().try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = c.lock().unwrap().kill();
}

/// After a failed restart: wait for `restart` (true) or `quit` / end of input (false) on the
/// console.
pub fn wait_for_restart() -> bool {
    let (tx, rx) = mpsc::channel();
    console_input(tx);
    loop {
        crate::repl::prompt();
        match rx.recv() {
            Ok(In::Console(l)) if l.trim() == "restart" => return true,
            Ok(In::Console(l)) if matches!(l.trim(), "quit" | "q" | "exit") => return false,
            Ok(In::Console(_)) => println!("type restart (or quit)"),
            _ => return false,
        }
    }
}

fn console_input(tx: mpsc::Sender<In>) {
    *CONSOLE.lock().unwrap() = Some(tx);
    CONSOLE_READER.call_once(|| {
        std::thread::spawn(|| {
            let send = |m: In| {
                if let Some(t) = CONSOLE.lock().unwrap().as_ref() {
                    let _ = t.send(m);
                }
            };
            let mut con = crate::console::Console::new();
            loop {
                // (prompt again once the previous command's report is printed)
                crate::console::wait_ready();
                match con.read() {
                    Some(line) => send(In::Console(line)),
                    None => break,
                }
            }
            send(In::ConsoleEof);
        });
    });
}

/// Serve `sim` (not started yet) on `addr` until the process is stopped, the viewer started
/// with `viewer` exits, or (with `console`: commands on stdin) the console quits. Returns true
/// when the console asked for a restart (the viewer has been closed; load the design again and
/// serve it anew).
pub fn serve(mut sim: Sim, addr: &str, viewer: Option<&Viewer>, console: bool) -> Result<bool, String> {
    let addr = if addr.contains(':') { addr.to_string() } else { format!("127.0.0.1:{addr}") };
    let listener = TcpListener::bind(&addr).map_err(|e| format!("cannot listen on {addr}: {e}"))?;
    // (port 0: the system picks one; report the real address)
    let addr = listener.local_addr().map_or(addr, |a| a.to_string());
    let mb: usize = std::env::var("FX_HISTORY_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(512);
    sim.history_start(mb << 20);
    sim.start();
    let st = sim.advance(Some(0));
    let pending = match viewer {
        Some(v) => Some(initial_items(&sim, &v.traces)?),
        None => None,
    };
    let precision = sim.precision();
    let mut ses = Session::from_sim(sim, precision, String::new());
    ses.mark_started();
    let mut srv = Server::new(ses);
    srv.wcp.pending = pending;
    if st != RunState::Paused {
        srv.status = Status::Finished;
        srv.ses.sim.finish();
        srv.ses.mark_finished();
    }
    srv.ses.sim.flush();
    // with a viewer and no console: run at once (the viewer shows the waves at the end, or
    // after AUTO_RUN when the simulation goes on)
    // (it starts when the viewer has referenced its signals: Surfer fetches values only
    // after a status event, so the run must end after it is listening)
    srv.auto_run = viewer.is_some() && !console && srv.status == Status::Paused;
    let (tx, rx) = mpsc::channel();
    let mut viewer_child: Option<Arc<Mutex<std::process::Child>>> = None;
    if let Some(v) = viewer {
        let wl = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("cannot listen for the viewer: {e}"))?;
        let port = wl.local_addr().map_err(|e| e.to_string())?.port();
        let child = std::process::Command::new(&v.bin)
            .arg(format!("cxxrtl+tcp://{addr}"))
            .args(["--wcp-initiate", &port.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot start '{}' ({e}); FX_SURFER selects the Surfer executable", v.bin))?;
        eprintln!("ferrooxide: debug server on {addr}, started {}", v.bin);
        let child = Arc::new(Mutex::new(child));
        viewer_child = Some(child.clone());
        let t = tx.clone();
        std::thread::spawn(move || loop {
            // (polled: the lock must stay free for a kill at restart)
            if !matches!(child.lock().unwrap().try_wait(), Ok(None)) {
                let _ = t.send(In::ViewerExited);
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        });
        let t = tx.clone();
        std::thread::spawn(move || {
            // one viewer connection
            if let Some(Ok(s)) = wl.incoming().next() {
                let _ = s.set_nodelay(true);
                let Ok(r) = s.try_clone() else { return };
                if t.send(In::WcpConnected(s)).is_ok() {
                    reader(r, t, In::WcpMsg, In::WcpClosed);
                }
            }
        });
    } else {
        eprintln!("ferrooxide: debug server on {addr}; connect with: surfer cxxrtl+tcp://{addr}");
    }
    if console {
        console_input(tx.clone());
        crate::repl::prompt();
    }
    std::thread::spawn(move || {
        for (id, s) in listener.incoming().flatten().enumerate() {
            let id = id as u32;
            let _ = s.set_nodelay(true);
            let Ok(r) = s.try_clone() else { continue };
            if tx.send(In::Connected(id, s)).is_err() {
                return;
            }
            let tx = tx.clone();
            std::thread::spawn(move || reader(r, tx, move |m| In::Msg(id, m), In::Closed(id)));
        }
    });
    loop {
        if srv.wcp.retry_at.is_some() {
            srv.wcp_poll();
        }
        let msg = if let Status::Running(_) = srv.status {
            match rx.try_recv() {
                Ok(m) => Some(m),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(false),
            }
        } else if let Some(t) = srv.wcp.retry_at {
            match rx.recv_timeout(t.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Ok(false),
            }
        } else {
            match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => return Ok(false),
            }
        };
        match msg {
            Some(In::Connected(id, s)) => {
                if let Some((_, old)) = srv.out.take() {
                    let _ = old.shutdown(std::net::Shutdown::Both);
                }
                srv.out = Some((id, s));
            }
            Some(In::Msg(id, text)) if srv.out.as_ref().is_some_and(|o| o.0 == id) => srv.handle(&text),
            Some(In::Closed(id)) if srv.out.as_ref().is_some_and(|o| o.0 == id) => srv.out = None,
            Some(In::WcpConnected(s)) => {
                srv.wcp.out = Some(s);
                srv.wcp_send("", json!({"type": "greeting", "version": "0", "commands": []}));
            }
            Some(In::WcpMsg(text)) => srv.wcp_handle(&text),
            Some(In::WcpClosed) => srv.wcp = Wcp::default(),
            Some(In::ViewerExited) | Some(In::ConsoleEof) => {
                if srv.status != Status::Finished {
                    srv.ses.finish();
                }
                return Ok(false);
            }
            Some(In::Console(line)) if line.trim() == "restart" => {
                if viewer_child.is_none() {
                    // (an external client would have to reconnect to a new server)
                    println!("restart: only with --surfer here; restart ferrooxide and reconnect");
                    crate::repl::prompt();
                    continue;
                }
                close_viewer(&mut srv, viewer_child.as_ref());
                return Ok(true);
            }
            Some(In::Console(line)) => {
                if !srv.console(&line) {
                    if srv.status != Status::Finished {
                        srv.ses.finish();
                    }
                    // quit: the viewer goes too
                    close_viewer(&mut srv, viewer_child.as_ref());
                    return Ok(false);
                }
            }
            Some(_) => {}
            None => srv.step(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}
