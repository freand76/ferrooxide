# ferrooxide — a fast 4-state, event-driven Verilog simulator in Rust

## Goals
- IEEE 1364-2005 Verilog first, then a pragmatic synthesizable/testbench subset of SystemVerilog (1800).
- Full 4-state semantics (0/1/X/Z), IEEE stratified event scheduler (active / inactive / NBA / postponed).
- **≥10x Icarus (vvp) throughput** on typical RTL + testbench workloads.
- Output-compatible with Icarus for `$display`-style testbenches → differential testing against `iverilog` is the main correctness oracle.

## Architecture

```
 .v/.sv files
     │  ferrooxide-front    preprocessor (`define/`ifdef/`include) → lexer → recursive-descent/Pratt parser → AST
     ▼
   AST
     │  ferrooxide-compile  elaboration: params, hierarchy flattening, port collapsing (aliasing),
     │                width/sign inference (IEEE context-determined rules), codegen
     ▼
  Program  (flat memory image + per-process bytecode + static fanout tables)
     │  ferrooxide-rt       event scheduler + bytecode VM (later: Cranelift JIT backend)
     ▼
  simulation
```

### Usage model
One command, no intermediate files: `ferrooxide tb.v dut.v` parses, elaborates, compiles to in-memory
bytecode (milliseconds) and simulates. Unlike `iverilog` + `vvp` there is no separate compile step.

### Key performance decisions
1. **Flat memory**: every signal, temp and constant lives in one `Vec<u64>`. A value of width W uses
   `n = ceil(W/64)` words of `aval` followed by `n` words of `bval` (VPI encoding: 00=0, 10=1, 01=Z, 11=X).
   Unused high bits are always kept zero → equality is a word compare.
2. **2-state fast path**: every op on ≤64-bit values checks `(b_a | b_b) == 0` once and then does plain
   machine arithmetic. X/Z handling is the slow path.
3. **Static everything**: temps are statically allocated per process (no allocation during simulation),
   event controls get static ids, and the signal→waiter fanout is a precomputed flat table. Waking a
   process = one compare + push.
4. **Port collapsing**: ports connected to whole signals alias the parent signal (no copy process).
5. **Processes are coroutines with a single `pc`** — suspension is free.
6. Planned (where the 10x really comes from):
   - **Cranelift JIT** of process bytecode (removes dispatch overhead, keeps 2-state fast path in registers).
   - **Levelized/static scheduling of combinational logic** (`assign`, `always @*`) inside clock domains:
     evaluate in topological order instead of through the event queue; fall back to events on loops.
   - **Clock-domain batching**: `always @(posedge clk)` processes with only NBAs are run as one fused function.
   - Timing wheel instead of a BTreeMap for the future-event queue.
   - Optional 2-state "shadow" for signals proven never to hold X (after reset).
   - Multi-threading of independent partitions (stretch goal).

### Learnings adopted from digsim-native
- Same value encoding (value word + undefined-bits word) and its `get_bits`/`set_bits` helpers (`ferrooxide-rt/src/ops.rs`).
- **Rank-ordered combinational settle** (digsim `levels.rs`): give every combinational process a rank
  above its drivers (Kahn longest path, SCCs/loops excluded), evaluate each dirty one once per settle in
  rank order, without events in between. Loops fall back to events. This gave digsim 45–100x and is
  the core of M4.
- Flip-flops update "in the next round" = our NBA region; group edge-triggered processes per clock edge.
- Whole-block "sweep" evaluation when most inputs changed (cheaper than tracking dirtiness).
- Profiling counters behind a cargo `prof` feature.
- Deterministic event order so engines can be diffed exactly (here: vs. Icarus).

## Milestones

| # | Content | Status |
|---|---------|--------|
| M0 | Workspace, plan, CLI, diff-test harness vs iverilog, benchmark | ✅ |
| M1 | Core Verilog: modules/ports (ANSI + non-ANSI), params/localparam + overrides, wire/reg/integer/time, vectors + memories, assign, initial/always, blocking/NBA (incl. `<= #d`), if/case/casez/casex/for/while/repeat/forever, `#`, `@`, `wait`, all operators w/ correct width & sign rules, functions/tasks (inlined), generate for/if/case, gate primitives, `$display/$write/$monitor/$strobe/$finish/$time/$random/$signed/$unsigned/$clog2/$readmemh/b/$fopen/$fdisplay` | ✅ initial version |
| M1b | VCD: `$dumpfile/$dumpvars/$dumpon/$dumpoff` | ✅ basic (dumps everything; scope/level args ignored) |
| M2 | `timescale`, reals, plusargs, hierarchical task calls, `$sformat`, `$fopen` modes | ✅ |
| M2b | Testbench concurrency: fork/join (+ SV join_any/join_none, disable fork), named events, `disable` of blocks/tasks across processes, automatic tasks/functions (per-call storage), Icarus wake order | ✅ (recursion: see notes) |
| M3 | Nets: multiple drivers with wire resolution, tri-state buses through inout ✅; wand/wor/tri0/tri1, strengths, `assign #d` inertial delays | partly |
| M4 | Performance I: levelized comb evaluation, fast ops, scratch temporaries | ✅ (timing wheel pending) |
| M5 | Performance II: Cranelift JIT backend | ✅ ≤64-bit ops, shared per shape (wide ops pending) |
| M6 | SystemVerilog: logic/bit/int types, always_comb/ff, `++/+=`, typedef/enum/struct (packed), packages, interfaces (basic), `$display` extensions | partly (lexing/simple types) |
| M7 | FST output, VPI-lite, better diagnostics | |
| M9 | CXXRTL Debug Server Protocol (CDSP): interactive debugging / live waveforms in CDSP clients | planned (see notes) |
| M10 | MCP server: the simulator as a tool for AI agents (load, run, inspect, force, waveforms, lint) | ✅ first version over stdio (`--mcp`); next: value history queries, resources, HTTP transport |
| M11 | Interactive mode (`-i`): run/step/print/list/set/force/release commands on stdin | ✅ first version (next: watch, breakpoints on conditions, waveform history, scopes) |
| M8 | Multi-threading: partition same-clock-edge and same-rank processes that write disjoint signals; thread pool per settle step, serial fallback for small buckets; deterministic output | |

## Road to full Verilog-2005 (notes)

Order of work: measure first (ivtest), then the groups below roughly in order of how often real
code needs them.

0. **Conformance measurement.** Run Icarus's regression suite *ivtest* (a few thousand small
   tests with expected output) through `tests/difftest.sh`-style comparison; track the pass rate
   and let the failures order the work below. Also chipsalliance *sv-tests* for parser coverage.
1. **Testbench concurrency** (M2b, done): fork branches are processes started by `Fork`
   (Icarus order: last branch first), joins count finished branches, `disable` uses a table of
   named-block code ranges (+ the branches forked inside them) and process generations to drop
   stale wake-ups, events are toggled variables, automatic subroutines get storage per call
   site. Waiters woken by one change run newest first (Icarus's LIFO waiting lists, also for
   edge groups, which alternate their member order). Remaining: recursion needs real call
   frames (a VM call stack and frame-relative slots in the JIT) instead of inlining; a fork
   branch restarted while still running (fork/join_none in a loop) should become a new
   process instance instead of restarting the old one.
2. **Structural RTL gaps**: multi-dimensional arrays (`reg [7:0] m [0:3][0:15]`),
   hierarchical references with genvar indices (`gen[i].u.sig`; constant indices work),
   real-valued function arguments in expressions everywhere. Done: arrays of instances,
   `defparam`, `` `default_nettype ``, implicit nets.
3. **Testbench I/O and system functions**: `$fread`, `$dist_*`, `%u`/`%z` in `$fscanf`;
   `$dumpvars` scope/level arguments and `$dumpall`/`$dumplimit`; force on array elements; real
   math (`$sqrt`, `$ln`, `$pow`, `$floor`, ...); `$countdrivers`, `$q_*` (rare). Done: file
   input, `$random`/`$urandom` (Icarus sequences), `$timeformat`/`$printtimescale`,
   `$writemem*`, force/release and assign/deassign.
4. **Gate level / netlists**: done: UDPs, net types, static drive strengths (per driver;
   the strongest wins per bit), pull gates, MOS switches, `tran`/`tranif` (islands solved in
   one process). Remaining: strength values on nets (`%v`, strength reduction through
   switches, `trireg` charge), gate and net delays (rise/fall/turn-off), inertial delays on
   `assign`, `specify` path delays and timing checks, SDF back-annotation.
5. **Tool interfaces and the rest**: VPI/PLI (cocotb and friends), Verilog-2001 configurations
   and libraries (`config`, `-y`/`-v` library search, `` `uselib ``), `$dumpports`, VCD of
   reals and events.

## M9: CXXRTL Debug Server Protocol (notes)

Spec: https://cxxrtl.org/protocol.html (v0.14.0, protocol version 0, still changing). Lets a
client (waveform viewer / debugger front end, e.g. Surfer; client support to be verified) attach to
a running simulation, run/pause it and query waveforms of any signal over any past interval.

**Protocol in short.** Byte stream (TCP or Unix socket), each message a UTF-8 JSON object
terminated by NUL. Greeting both ways (`version: 0`; the server lists its commands, events and
`item_values_encoding: ["base64(u32)"]`). Commands: `list_scopes`, `list_items` (scope = direct
children only; ids are space-separated hierarchical names, root scope `""`), `reference_items`
(bind a name to a list of items / memory row ranges), `query_interval` (samples with item values
and diagnostics for an inclusive time interval, optionally collapsed to one sample per time),
`get_simulation_status`, `run_simulation` (`until_time`, `until_diagnostics`, async),
`pause_simulation`. Events: `simulation_paused`, `simulation_finished`. Times are strings
`"<s>.<fs>"`. Values: per item little-endian u32 words, concatenated, base64. Diagnostics:
`print`/`break`/`assert`/`assume` with source location. No value setting yet.

**Design for ferrooxide.**
1. *Server thread + simulation thread.* `--debug-server tcp:HOST:PORT` (or `unix:PATH`). The
   simulation thread runs time steps and checks a command channel between time steps (cheap
   atomic flag), so run/pause/until_time need no locking inside the hot loop. Paused = the
   simulation thread blocks on the channel.
2. *Hierarchy.* Scopes from instance paths (`tb.u1.sub` → `"tb u1 sub"`), items from signal
   names (all `prog.names` including port aliases); `type` node/memory, `width`, `lsb_at` from the
   declared range, `input`/`output` from port direction (keep it in `SignalInfo`), memories with
   `depth`/`zero_at`, `src` from file:line (keep declaration locations), `settable: false`.
3. *History for query_interval.* Phase 1: an in-memory change log (time, signal, value) fed by
   the same per-time-step dirty list as VCD (`notify` already marks changed signals when dumping),
   for all non-array signals, with a memory budget (drop or compress old history; follow the
   project's memory rules). Memories: log element writes (`notify_elem` knows the element).
   Phase 2 (CXXRTL's approach): periodic checkpoints of the whole simulation state (`mem`,
   process states, queues, NBA buffer, RNG, file handles muted) and deterministic re-simulation
   from the nearest checkpoint; only cheap checkpoints are stored, values are recomputed on demand.
4. *Values.* `base64(u32)` is two-state: x/z bits are sent as the value plane (x→1, z→0) for now.
   Propose/negotiate a four-state encoding (value + unknown planes, e.g. a `base64(u32,4state)`
   feature) since 4-state is the point of this simulator; check what clients accept.
5. *Diagnostics.* `$display`/`$write` output → `print` (with time and src), `$finish`/`$stop` →
   `break`, SystemVerilog immediate assertions later → `assert`. Keep them in the history so
   `query_interval` can return them; `run_simulation until_diagnostics` pauses on them.
6. *Time.* ticks × 10^(15 + precision) femtoseconds; `run_simulation until_time` stops before the
   first time step after the target and emits `simulation_paused` (cause `until_time`).
7. *Later.* Setting values (when the spec adds it: force-like writes through the force mask),
   breakpoints on signal conditions, scope/item attributes (`(* ... *)` are currently dropped by
   the lexer and would need keeping), a VCD/FST import mode as a read-only server.

Open questions: exact client behaviour (Surfer's CDSP support and which encodings it accepts),
how large a history is useful by default, and whether to expose x/z before the spec defines it.

## M10: MCP server (notes)

The Model Context Protocol (https://modelcontextprotocol.io) lets an AI agent (Claude Code,
other MCP clients) call tools of a server. Exposing ferrooxide this way lets an agent debug a
design by running and probing it instead of reading logs and dumping huge VCDs.

**Transport.** `ferrooxide --mcp` speaks JSON-RPC 2.0 over stdio (the standard local transport;
streamable HTTP later). No design on the command line needed: the agent loads one through a tool.
Use an existing Rust MCP SDK if it is mature (e.g. the official `rmcp` crate), else implement
the small subset (initialize, tools/list, tools/call, resources) by hand.

**Shares the M9 core.** The interactive mode (M11) already has the basic pieces: the simulator
can be started, advanced to a time and stopped between time steps (`Sim::start/advance/finish`),
and signals read, deposited, forced and released by name; an MCP or CDSP front end can drive the
same API from a server thread. Simulation thread + command channel checked between time steps, the
hierarchy model, the change-log history and diagnostics capture are the same as for CDSP; MCP and
CDSP are two front ends. Build that core once (M9 step 1-3), then add whichever front end first.

**Tools (first cut).**
- `load_design {files, defines, include_dirs, top, plusargs, dialect}` → top scopes, warnings and
  compile errors with file:line (an agent's edit-compile loop wants these structured).
- `lint` → incomplete sensitivity lists etc. (existing `--lint`).
- `run {until_time | for_time | until_condition, max_steps}` → stop reason, current time,
  `$display` output produced meanwhile (truncated, with a count).
- `list_scopes {scope}` / `list_signals {scope, pattern}` → names, widths, directions, kinds.
- `get_values {signals, radix}` → current 4-state values (x/z shown as in `$display`).
- `get_history {signals, from, to, max_changes}` → value changes (from the change log), so an
  agent can ask "when did `state` change before `err` rose?" without a VCD.
- `set_value {signal, value}` (deposit), `force {signal, value}`, `release {signal}` → existing
  force masks.
- `add_breakpoint {condition}` (expression over signals, compiled like a `wait`), `list` /
  `remove_breakpoint`; `run` stops on them.
- `dump_vcd {file, scopes, from, to}` → write a VCD for a human (or a viewer) from history.
- `reset` / `restart` (re-elaborate) and `status` (time, finished, memory use).

**Resources.** Source files of the loaded design, the elaborated hierarchy, the simulation log.

**Agent-oriented details.** Every result small and bounded (agents pay per token): truncation
with counts, pattern filters, no unbounded dumps. Errors as structured content with locations.
Deterministic (same commands → same results), long runs cancellable, memory budget for history
(same rules as the rest: an out-of-memory crash takes the client down with it).

Open questions: one server per design or several sessions; whether conditions/breakpoints use
the normal expression compiler (likely: compile to a small process that wakes on the signals);
how to present large memories.

## Testing strategy
- `tests/verilog/*.v`: self-contained testbenches. `tests/difftest.sh` runs each through `iverilog`+`vvp`
  and `ferrooxide` and diffs stdout. `cargo test` runs the same comparison (skipped if iverilog is absent).
- `bench/`: synthetic and realistic designs; `bench/run.sh` reports wall time for both simulators.

## Performance status (CPU cycles, `perf stat`, same machine)

| benchmark | Icarus | ferrooxide JIT | ferrooxide `--no-jit` | speed-up |
|-----------|--------|----------------|-----------------------|----------|
| `bench/cpuish.v` (32 lanes LFSR+ALU, 200k cycles) | 33.6 G | 2.96 G | 5.0 G | **11.3x** |
| `bench/mixed.v` (comb chain, 128-bit datapath, memory, FSM, 300k cycles) | 13.5 G | 0.66 G | 1.4 G | **20.3x** |

With a full VCD dump (all signals; wall time, same machine, 2026-11-13):

| benchmark | Icarus | Icarus + `$dumpvars` | ferrooxide | ferrooxide `--vcd` | VCD size |
|-----------|--------|----------------------|------------|--------------------|----------|
| `bench/cpuish.v` | 14.3 s | 19.5 s | 0.73 s | 1.95 s | 707 MB (Icarus 720 MB) |
| `bench/mixed.v` | 3.1 s | 7.4 s | 0.15 s | 0.51 s | 175 MB (Icarus 308 MB) |

Dumping everything costs ferrooxide 2.7-3.4x (mostly formatting and writing the text); the lead
over Icarus with a dump is 10-14x. `--trace REGEX` limits the cost to the selected signals.

What got us here (in order of impact):
1. Lowering to single-word fast ops with the 2-state check inline (`rt/fast.rs`).
2. Cranelift JIT per *process shape*: identical processes of different instances share one native
   function and get their slots/signals/controls from a per-process table. Without sharing, 32 lane
   copies thrashed the instruction cache (586M i-cache misses → 2M).
3. Shared scratch region for temporaries (dead at every suspension point): L1d misses 161M → 42M.
4. Rank-ordered combinational settle (from digsim-native), as a dirty bitset in rank order, with a
   separate combinational fan-out list that needs no per-process state.
5. Constant-index array accesses resolved at compile time; inline `Wait` and inline NBA queue
   appends in JIT code.
6. Element-level sensitivity for constant-index array accesses (virtual element signals, element
   granular rank graph): mixed.v 27.6M → 8.4M activations.
7. Wide (≤1024-bit) ops compiled natively (unrolled word loops), concatenation as bit-field
   inserts, and scratch temporaries promoted to SSA variables (registers) unless a generic
   callback reads them.

8. Ports connected to constant-index array elements alias the element (no copy process).
9. Fusion of combinational chains (`rt/fuse.rs`): a pure combinational process whose only
   combinational consumer is B is merged into B (cpuish 26.0M → 19.6M activations, mixed
   3.6M → 1.2M).
10. Fast paths for variable-index memory reads/writes, part-select stores, signed compares,
    logical not and wire resolution; edge groups (clocked processes with identical triggers
    run as one activation instead of one per process).

Scheduling order: wakes are ordered FIFO (sequence numbers) between active processes and the
combinational settle, matching Icarus in the usual same-time-step testbench races
(tests/verilog/race.v).

Next performance steps: ports connected to part selects as aliases, fusing sibling processes
with the same inputs, signal values cached in registers within an activation, timing wheel,
`--threads` (partitioned same-edge/same-rank parallelism, see M8).

## Known limitations of the current skeleton
- Event expressions must be whole signals (`@(posedge clk)`, not `@(posedge v[3])`).
- Wide (>64 bit) arithmetic uses a slow generic path.
- No real numbers, fork/join runs branches sequentially.
