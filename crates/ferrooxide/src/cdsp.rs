//! CXXRTL Debug Server Protocol server (`--cdsp [HOST:]PORT`): waveform viewers such as Surfer
//! (`surfer cxxrtl+tcp://HOST:PORT`) attach to the running simulation, run and pause it and query
//! the value history of any signal. Spec: https://cxxrtl.org/protocol.html (protocol version 0).
//!
//! Messages are JSON objects terminated by NUL. One client at a time (a new connection replaces
//! the old one); the simulation keeps its state across connections. Values are sent as
//! `base64(u32)`, which is two-state: x is sent as 1, z as 0. Arrays and reals are not listed yet.

use ferrooxide_rt::{RunState, Sim};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;

enum In {
    Connected(u32, TcpStream),
    Msg(u32, String),
    Closed(u32),
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
    sim: Sim,
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

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

fn parent(id: &str) -> &str {
    id.rfind(' ').map_or("", |i| &id[..i])
}

impl Server {
    fn new(sim: Sim) -> Server {
        let fs_per_tick = 10u128.pow((15 + sim.precision()).clamp(0, 30) as u32);
        let mut scopes = BTreeSet::new();
        scopes.insert(String::new());
        let mut items = BTreeMap::new();
        for (name, sig) in sim.signal_names() {
            let info = sim.signal_info(*sig);
            if info.depth > 0 || info.real || name.contains(' ') {
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
        Server { sim, status: Status::Paused, fs_per_tick, scopes, items, refs: HashMap::new(), out: None, warned_full: false }
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
        if let Some((_, s)) = &mut self.out {
            let mut b = serde_json::to_vec(&v).unwrap();
            b.push(0);
            if s.write_all(&b).is_err() {
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
            }
            "query_interval" => self.query_interval(m),
            "get_simulation_status" => {
                let mut r = json!({"type": "response", "command": cmd, "latest_time": self.time_str(self.sim.now())});
                r["status"] = json!(match self.status {
                    Status::Paused => "paused",
                    Status::Running(_) => "running",
                    Status::Finished => "finished",
                });
                if self.status == Status::Paused {
                    let next = self.sim.next_event_time().unwrap_or(self.sim.now());
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
                if let Status::Running(_) = self.status {
                    self.status = Status::Paused;
                    self.sim.flush();
                }
                let t = self.time_str(self.sim.now());
                self.send(json!({"type": "response", "command": cmd, "time": t}));
            }
            _ => self.error("invalid_command", format!("unknown command '{cmd}'")),
        }
    }

    fn query_interval(&mut self, m: &Value) {
        let (Some(b), Some(e)) = (self.parse_time(&m["interval"][0]), self.parse_time(&m["interval"][1])) else {
            return self.error("invalid_args", format!("bad interval {}", m["interval"]));
        };
        let now = self.sim.now();
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
        let h = self.sim.history().unwrap();
        let sigs: Vec<(u32, u32)> =
            ids.into_iter().flatten().map(|id| (self.items[id].sig, self.items[id].width)).collect();
        // sample times: the interval start, then every change of a referenced item
        let mut times = vec![b];
        for &(s, _) in &sigs {
            let ts = &h.times[s as usize];
            let lo = ts.partition_point(|&t| t <= b);
            let hi = ts.partition_point(|&t| t <= e);
            times.extend_from_slice(&ts[lo..hi]);
        }
        times.sort_unstable();
        times.dedup();
        let mut cur: Vec<Option<usize>> = sigs.iter().map(|&(s, _)| h.index_at(s, b)).collect();
        let mut samples = Vec::with_capacity(times.len());
        let mut bytes = Vec::new();
        for &t in &times {
            let mut sample = json!({"time": self.time_str(t)});
            if values {
                bytes.clear();
                for (k, &(s, w)) in sigs.iter().enumerate() {
                    let ts = &h.times[s as usize];
                    let mut i = cur[k].map_or(0, |i| i + 1);
                    while i < ts.len() && ts[i] <= t {
                        i += 1;
                    }
                    cur[k] = i.checked_sub(1);
                    let n32 = w.div_ceil(32) as usize;
                    let planes = cur[k].map(|i| h.value(s, i));
                    for j in 0..n32 {
                        let word = planes.map_or(0, |p| (p[j / 2] >> (32 * (j % 2))) as u32);
                        bytes.extend_from_slice(&word.to_le_bytes());
                    }
                }
                sample["item_values"] = json!(base64(&bytes));
            }
            if m["diagnostics"].as_bool() == Some(true) {
                sample["diagnostics"] = json!([]);
            }
            samples.push(sample);
        }
        self.send(json!({"type": "response", "command": "query_interval", "samples": samples}));
    }

    /// Run one time step of a running simulation; emits the event when it stops.
    fn step(&mut self) {
        let Status::Running(until) = self.status else { return };
        let st = match self.sim.next_event_time() {
            Some(t) if until.is_none_or(|u| t <= u) => self.sim.advance(Some(t)),
            Some(_) => {
                let st = self.sim.advance(until);
                if st == RunState::Paused {
                    self.status = Status::Paused;
                    self.sim.flush();
                    let t = self.time_str(self.sim.now());
                    self.send(json!({"type": "event", "event": "simulation_paused", "time": t, "cause": "until_time"}));
                    return;
                }
                st
            }
            None => RunState::Idle,
        };
        if let Some(h) = self.sim.history() {
            if let (Some(t), false) = (h.full_at, self.warned_full) {
                self.warned_full = true;
                eprintln!("ferrooxide: value history full at time {t} (FX_HISTORY_MB); later changes are not recorded");
            }
        }
        if st != RunState::Paused {
            self.status = Status::Finished;
            self.sim.finish();
            let t = self.time_str(self.sim.now());
            self.send(json!({"type": "event", "event": "simulation_finished", "time": t}));
        }
    }
}

fn reader(id: u32, mut s: TcpStream, tx: mpsc::Sender<In>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1 << 16];
    loop {
        let n = match s.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        while let Some(i) = buf.iter().position(|&c| c == 0) {
            let msg = String::from_utf8_lossy(&buf[..i]).into_owned();
            buf.drain(..=i);
            if tx.send(In::Msg(id, msg)).is_err() {
                return;
            }
        }
    }
    let _ = tx.send(In::Closed(id));
}

/// Serve `sim` (not started yet) on `addr` until the process is stopped.
pub fn serve(mut sim: Sim, addr: &str) -> Result<(), String> {
    let addr = if addr.contains(':') { addr.to_string() } else { format!("127.0.0.1:{addr}") };
    let listener = TcpListener::bind(&addr).map_err(|e| format!("cannot listen on {addr}: {e}"))?;
    // (port 0: the system picks one; report the real address)
    let addr = listener.local_addr().map_or(addr, |a| a.to_string());
    let mb: usize = std::env::var("FX_HISTORY_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(512);
    sim.history_start(mb << 20);
    sim.start();
    let st = sim.advance(Some(0));
    let mut srv = Server::new(sim);
    if st != RunState::Paused {
        srv.status = Status::Finished;
        srv.sim.finish();
    }
    srv.sim.flush();
    eprintln!("ferrooxide: debug server on {addr}; connect with: surfer cxxrtl+tcp://{addr}");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for (id, s) in listener.incoming().flatten().enumerate() {
            let id = id as u32;
            let _ = s.set_nodelay(true);
            let Ok(r) = s.try_clone() else { continue };
            if tx.send(In::Connected(id, s)).is_err() {
                return;
            }
            let tx = tx.clone();
            std::thread::spawn(move || reader(id, r, tx));
        }
    });
    loop {
        let msg = if let Status::Running(_) = srv.status {
            match rx.try_recv() {
                Ok(m) => Some(m),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        } else {
            match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => return Ok(()),
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
