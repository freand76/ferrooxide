#!/usr/bin/env python3
"""Stand-in for Surfer in the --surfer test: started as `surfer cxxrtl+tcp://ADDR
--wcp-initiate PORT`. Answers WCP like Surfer (greeting; add_items fails until the CXXRTL
source is connected), runs the simulation to the end over CDSP and logs the WCP commands it
received to $FAKE_SURFER_LOG. With FAKE_SURFER_MODE=watch it only connects, writes "ready" and
then logs the CDSP events and WCP commands it receives until the server closes."""
import json, os, select, socket, sys

src, port = sys.argv[1], int(sys.argv[3])
log = open(os.environ["FAKE_SURFER_LOG"], "w")

class Conn:
    def __init__(self, addr):
        self.s, self.buf = socket.create_connection(addr), b""
    def send(self, m):
        self.s.sendall(json.dumps(m).encode() + b"\0")
    def recv(self):
        while b"\0" not in self.buf:
            d = self.s.recv(65536)
            if not d:
                raise SystemExit("connection closed")
            self.buf += d
        i = self.buf.index(b"\0")
        m, self.buf = json.loads(self.buf[:i]), self.buf[i + 1:]
        return m

wcp = Conn(("127.0.0.1", port))
assert wcp.recv()["type"] == "greeting"
wcp.send({"type": "greeting", "version": "0", "commands": ["add_items", "add_markers"]})

if os.environ.get("FAKE_SURFER_MODE") == "watch":
    host, p = src.removeprefix("cxxrtl+tcp://").rsplit(":", 1)
    cx = Conn((host, int(p)))
    cx.send({"type": "greeting", "version": 0})
    cx.recv()
    m = wcp.recv()
    wcp.send({"type": "response", "command": "add_items", "ids": [0]})
    log.write("ready\n")
    log.flush()
    conns = {cx.s: cx, wcp.s: wcp}
    while True:
        for c in [c for c in conns.values() if b"\0" in c.buf] or [conns[r] for r in select.select(list(conns), [], [])[0]]:
            if b"\0" not in c.buf:
                d = c.s.recv(65536)
                if not d:
                    log.write("closed\n")
                    raise SystemExit
                c.buf += d
                continue
            m = c.recv()
            if c is cx:
                log.write(f"cdsp {m.get('event', m.get('command'))} {m.get('time', '')}\n")
            else:
                log.write(f"wcp {m['command']} {m.get('timestamp', m.get('markers', ''))}\n")
                wcp.send({"type": "response", "command": "ack"})
            log.flush()

m = wcp.recv()
log.write(f"wcp {m['command']} {m['items']} (no waveform yet)\n")
wcp.send({"type": "error", "error": "add_items", "arguments": [], "message": "Can't add items. No waveform loaded"})

host, p = src.removeprefix("cxxrtl+tcp://").rsplit(":", 1)
cx = Conn((host, int(p)))
cx.send({"type": "greeting", "version": 0})
cx.recv()
m = wcp.recv()
log.write(f"wcp {m['command']} {m['items']}\n")
wcp.send({"type": "response", "command": "add_items", "ids": list(range(len(m["items"])))})

cx.send({"type": "command", "command": "run_simulation", "until_time": None, "until_diagnostics": [], "sample_item_values": True})
cx.recv()
log.write(f"cdsp {cx.recv()['event']}\n")
m = wcp.recv()
log.write(f"wcp {m['command']} {m['markers']}\n")
wcp.send({"type": "response", "command": "add_markers", "ids": [99]})
log.close()
