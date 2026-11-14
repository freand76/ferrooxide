#!/usr/bin/env python3
"""Stand-in for Surfer in the --surfer test: started as `surfer cxxrtl+tcp://ADDR
--wcp-initiate PORT`. Answers WCP like Surfer (greeting; add_items fails until the CXXRTL
source is connected), runs the simulation to the end over CDSP and logs the WCP commands it
received to $FAKE_SURFER_LOG."""
import json, os, socket, sys

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
