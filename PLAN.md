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
| M2b | Testbench concurrency: fork/join (+ SV join_any/join_none, disable fork), named events, `disable` of blocks/tasks across processes, automatic tasks/functions (per-call storage, recursion), Icarus wake order | ✅ |
| M3 | Nets: multiple drivers, tri-state buses, wand/wor/tri0/tri1/supply, static drive strengths, tran islands, inertial delays (rise/fall/turn-off), specify path delays (-gspecify) | ✅ (strength values on nets / `%v`: pending) |
| M4 | Performance I: levelized comb evaluation, fast ops, scratch temporaries | ✅ (timing wheel pending) |
| M5 | Performance II: Cranelift JIT backend | ✅ ≤64-bit ops, shared per shape (wide ops pending) |
| M6 | SystemVerilog: logic/bit/int types, always_comb/ff, `++/+=`, typedef/enum/struct (packed), packages, interfaces (basic), `$display` extensions | typedef, enum, packed struct/union, casts, assignment patterns, packages/import, `$unit` done (parser-level, `svtypes.rs`); next: see "SystemVerilog status" |
| M7 | FST output, VPI-lite, better diagnostics | |
| M9 | CXXRTL Debug Server Protocol (CDSP): interactive debugging / live waveforms in CDSP clients | ✅ first version (`--cdsp`): hierarchy, references, history queries, run/pause; `--surfer` (starts Surfer, adds signals over WCP); reals; `-i` console next to the viewer (cursor follows run/step/breakpoints); next: arrays, port directions, diagnostics, goto_declaration |
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
   edge groups, which alternate their member order). Recursion (done): a call of a function
   that is being inlined becomes a subroutine call — `SaveFrame` copies the function's
   variables and the process's temporaries to a per-process frame stack, `CallSub`/`RetSub`
   jump and return (processes with recursion keep no temporaries in JIT registers; depth
   limit 100000 frames). `?:` with calls in its branches evaluates only the selected one.
   Remaining: a fork branch restarted while still running (fork/join_none in a loop) should
   become a new process instance instead of restarting the old one.
2. **Structural RTL gaps**: multi-dimensional arrays as ports and whole-array assignments,
   real-valued function arguments in expressions everywhere. Done: arrays of instances,
   `defparam`, `` `default_nettype ``, implicit nets, multi-dimensional unpacked arrays and
   packed vectors (stored flat; index chains become one index/select), hierarchical names
   with genvar indices.
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
client (waveform viewer / debugger front end; Surfer has a client, see below) attach to
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

**Status (first version, `--cdsp`).** `cdsp.rs`: listener thread, one reader thread per
connection, the simulation thread owns `Sim` and steps one time step at a time while running
(checking the command channel between steps). History: `rt/history.rs`, one sequential log
(signal id + planes packed to the width, 6 bytes per 1-bit change) fed from the dirty list at
the end of each time step (`Sim::history_start`; `track` replaces `dumping` as the dirty-list
switch), budget `FX_HISTORY_MB`; per-signal indexes are built on demand for the queried
signals. Running: `advance_steps` chunks sized to 10-40 ms. Cost on `bench/cpuish.v` (all
signals toggle): 0.64 s plain, 1.1-1.3 s with full history (290 MB); a query of 10 signals
over the whole run (400k samples) about 0.85 s, the first one 0.45 s more (indexing). Samples are sent at the interval
start and at each change of a referenced item. Reals are 64-bit items (IEEE double bits).
Not yet: arrays (memory items, row ranges; Surfer designates every item as a node, so a memory
would make its whole reference fail), port directions (`input`/`output` are false; needs direction in `SignalInfo`), `src`,
diagnostics (`$display` → `print`, `until_diagnostics`), the WCP pairing below.

**Surfer as client** (checked in its source, 2026-11-03 snapshot, `libsurfer/src/cxxrtl*`).
Connect with `surfer cxxrtl+tcp://HOST:PORT` (TCP only in native builds). What it sends, and so
what a server must handle:
- greeting `version: 0`; the server greeting needs `commands`, `events`, `features` (contents
  of `features` ignored).
- `list_scopes {scope: null}`: expects *all* scopes in one flat map (space-separated names);
  Surfer builds the tree itself. `list_items` with `null` (all items) and per scope; only
  `width` is read, unknown fields are ignored (send the full spec fields anyway).
- `reference_items "ALL_VARIABLES"` with `[[name], ...]`, re-sent whenever a signal is added.
- `query_interval`: always `[0, latest_time]`, `collapse: true`, `base64(u32)`, no
  diagnostics, re-sent after every status change. No incremental fetch: each pause transfers
  the whole history of the displayed signals. Surfer looks up the latest sample <= t, so
  samples only at times where a referenced item changed are enough.
- `run_simulation`: the play button asks for `until_time = latest + 100 ns` (hard-coded),
  `sample_item_values: true`, no diagnostics. `pause_simulation` expects `{time}` back.
  `get_simulation_status` → `{status, latest_time}`; events `simulation_paused` (with cause),
  `simulation_finished`. No polling while running: the view updates at events and pauses.
- Responses carry no ids; Surfer pairs them FIFO with its requests (an error pops one too), so
  the server must answer strictly in order.
- Values are decoded as unsigned integers: two-state only, x/z are lost. Value setting and
  diagnostics are not used.

**WCP: driving the viewer.** Surfer's Waveform Control Protocol (`surfer-wcp`, unfinished,
NUL-terminated JSON like CDSP) lets a tool control the viewer: `add_items` (signals or scopes,
recursive), `set_cursor`, `set_viewport_to/range`, `add_markers`, `load`, `reload`, `clear`,
`zoom_to_fit`, `set_item_color`, `get_item_list/info`, `remove_items`, `focus_item`. Events from
Surfer: `waveforms_loaded`, and, if the tool lists them in its greeting, `goto_declaration`,
`add_drivers`, `add_loads` (requests to the tool). `surfer --wcp-initiate PORT` makes Surfer
connect to a listening tool (always 127.0.0.1). WCP `load` accepts only files and URLs, not a
`cxxrtl+tcp://` source, and `waveforms_loaded` is only sent for file sources.

**Pairing (`--surfer`, first version done).** ferrooxide opens a CDSP and a WCP listener and
starts `surfer cxxrtl+tcp://127.0.0.1:<cdsp> --wcp-initiate <wcp>`. After Surfer's WCP greeting
it sends `add_items` (dotted names), retried every 250 ms while Surfer answers "No waveform
loaded"; `$finish` gets a marker (WCP times are femtoseconds for a CXXRTL source); ferrooxide
exits with Surfer. Tested with a stand-in (`tests/cdsp/fake_surfer.py`), not yet with Surfer
itself. Still to do; it then pushes views: `--trace REGEX` signals via `add_items`, `set_cursor` at breakpoints and `$stop`,
`$display`/assertions as markers. It answers `goto_declaration` from declaration locations and
`add_drivers`/`add_loads` from the elaborated netlist (something a VCD-only viewer cannot do).
Without a server Surfer also reads `--vcd` files directly. Both protocols still change.

**Upstream proposals.** A four-state value encoding (value + unknown planes) in CDSP and in
Surfer's decoder; incremental `query_interval` (only the new interval after a pause); a
configurable run step instead of the fixed 100 ns.

Open questions: how large a history is useful by default (it is re-sent at every pause), and
whether to expose x/z before the spec defines it.

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

## Status and next steps (2026-11-14, updated after the debug server)

Icarus regression suite (ivtest, normal tests): ~88% pass. Real-design benchmark: output
identical to Icarus, 10-13x faster. Interactive mode, MCP server, `--vcd/--trace`, the CDSP
debug server with value history and the Surfer pairing (`--cdsp`, `--surfer`, `-i` next to
the viewer) work.

Next, roughly in order (see "Architecture changes ahead" below for why this order):
1. **SystemVerilog RTL items that fit the current design**: unpacked dimensions in typedefs,
   `timeunit`/`timeprecision`, file-scope variables, `unique`/`priority` warnings,
   `$countones`/`$onehot`/`$onehot0`, arrays of interfaces. (Done: the `assign z = a;`
   alias, see the known difference below.)
2. **Types in elaboration** (A1 below), with `parameter type`.
3. **Heap objects** (A2): `string`, queues, dynamic and associative arrays.
4. **Dynamic processes, subroutine calls, dynamic waiters** (A3); fixes fork/join_none in a
   loop. Then classes and `program`; VPI (A4) on the same base.
5. **Remaining ivtest failures** (`tests/ivtest.py ... --show 40` groups them): compiler
   warnings Icarus prints (port coercion to inout, constant out-of-range selects, `@*` without
   sensitivities), `%v` strength display (needs strength values on nets), `$countdrivers`,
   `$fread`, `%u`/`%z` in `$scanf`, event-controlled nonblocking assignments updating at the
   trigger (ICARUS_NOTES 3), specify paths on inout ports and conditional paths, force on array
   elements, generate blocks referenced before their definition.
6. **Debug server follow-ups** (M9/M10): try `--surfer` with Surfer itself; MCP over HTTP
   (attach to a running simulation) and MCP history queries on the CDSP value history; arrays
   in CDSP (per-element items for small memories), `$display` as diagnostics, source
   locations (`goto_declaration`).
7. **Performance** (in parallel with the above): wide (>64-bit) ops in the JIT, a timing
   wheel; multi-threading (M8) last (A5).
8. **Outputs**: FST, VCD of arrays/reals/events, `$dumpvars` scope/level arguments.

Known difference (found 2026-11-14): the reference propagates some net updates immediately,
inside the writing process (`a = 1; $display(z)` shows the new value). Done: a plain `assign
z = a;` (also `{a}`, a constant-index element, `#0`, `wire z = a;`, chains, `logic` variables)
makes `z` a view of `a` (`alias_assigns`), unless `z` has other drivers, is a special net
type, has a strength, a link or is forced. Not yet: nets with several drivers, `wand`/`tri1`
and strength drivers (resolved immediately there, in the settle here), and `assign z = m[i]`
with a run-time index. Not immediate there either: logic (`a & b`), part selects, widening.
Interactive/MCP `force` of an aliased net forces its source.

Known limitations: recursion copies frames (fine for testbench code, slow for deep recursion);
multi-dimensional arrays as ports or whole-array assignments are not supported.

## Architecture changes ahead (review 2026-11-14)

What we know now that we did not at the start: RTL, performance and the debug front ends fit
the current design; SystemVerilog testbench features and VPI do not. The design was built
around two simplifications: types are resolved in the parser, and everything (processes,
storage, fan-out) is static.

**Fits as is.** Wide JIT ops, timing wheel (`future` is a `BTreeMap` behind a few calls),
part-select port aliases, `assign z = a` aliasing (one more port-alias case), strength values
(per-bit strength data next to the value, only on nets with resolve processes / tran
islands; the value encoding stays), FST, `$dumpvars` arguments, CDSP/MCP history and
breakpoints between time steps, the remaining Verilog-2005 gaps.

**A1. Types in elaboration.** `svtypes.rs` lowers every type to a flat vector in the parser,
and the compiler knows an expression only as (slot, width, signed) plus `is_real`. That cannot
express `parameter type T` (known per instance), unpacked structs (or structs with reals /
strings), `string`, queues, dynamic/associative arrays or class handles, which all have their
own operator rules. Change: the parser keeps only *which* names are types (needed to parse
`T x;`); a `Type` enum in the compiler (vector, real, string, queue(T), darray(T), assoc(K,T),
class(id), unpacked struct, ...) carries the meaning; packed types are still lowered to flat
vectors at the end, so runtime and JIT are unchanged for RTL. Prerequisite for most of the
remaining SystemVerilog. Good moment to split elaboration (types, nets/drivers, hierarchy) out
of `compile/lib.rs`.

**A2. Heap objects.** Strings, queues, dynamic and associative arrays live in a heap of
objects, referred to by handles stored in the flat memory (one value word; a null handle is
0). Ops on them are generic-path instructions at first. No dynamic processes needed, so this
can come right after A1.

**A3. Dynamic processes, calls and waiters.** Today a process is a fixed `ProcDef` with one
`ProcRt`, fork branches are fixed processes that get restarted (hence the open fork/join_none
in a loop), automatic storage is per call site, functions are inlined (recursion copies
frames), and fan-out is a fixed range of `(proc, ctl)` entries. Classes, fork in a loop, VPI
callbacks and MCP conditional breakpoints all need the same three things:
- process instances created at run time from a `ProcDef` template, with their own frame for
  automatic storage. The JIT already gets slots/signals/controls through a per-process table
  (`ProcFn(sim, mem, pc, table, hot)`), so an instance is a new table, no recompilation;
- real subroutine calls (methods through a handle, virtual dispatch); also usable for large
  functions called from many places instead of inlining them everywhere;
- dynamic waiters: a per-signal list next to the static fan-out, for waits whose target is
  only known at run time (`@(obj.ev)`, `vif.clk`, `cbValueChange`, breakpoint conditions).
  The static path stays the fast path; a per-signal flag says the list is non-empty.

**A4. VPI** (cocotb): handle tree from the hierarchy model (as for CDSP), get/put value
through the force-mask path, value-change callbacks as dynamic waiters (A3), region hooks in
the scheduler (ReadWriteSynch / ReadOnlySynch) and an `AfterDelay` entry in the future queue.

**A5. Multi-threading costs more than M8 suggests.** Absolute shared scratch slots, one NBA
queue in `Hot`, one wake buffer, and exact event order to match. Needs per-thread scratch (a
base pointer in the JIT ABI), per-thread NBA queues merged in a fixed order, parallel work
only for processes without system tasks. Pays off only on large designs: last of the
performance items.

**Watch.** Elaboration and codegen run per instance (only the JIT shares code between
instances); measure compile time and memory on a large netlist before assuming it scales.

## SystemVerilog status (2026-11-14)

Done (resolved in the parser, `crates/ferrooxide-front/src/svtypes.rs`): `typedef` (module and
file scope), `enum` (constants as localparams of the base type), packed `struct`/`union`
(struct-typed values are flat vectors; member/index/select chains become one `[off +: w]`
select, also at run-time indexes and as lvalues), casts `T'(e)`, `N'(e)`, `signed'(e)`,
`$bits(T)`, assignment patterns (positional, and beyond the reference: named and `default:`),
packages with `import p::*` / `import p::x` (module, header, file scope) and `pkg::name`.
Package items are copied into each module that uses the package (the module's own names win).
Lint reports names that are SystemVerilog keywords.

Since then: interfaces and modports (interface ports alias the connected instance's scope),
enum methods, array query functions, 2-state types (`bit`/`int`/...: start at 0, stores turn
x/z into 0; `UnOp::To2`), `always_comb`/`always_latch` sensitivity through called functions,
`` `__LINE__ `` in macros, `$isunknown`. SystemVerilog list now 251 of 700 (35.9%), main list
unchanged at 1337 of 1526.

Regression suite, SystemVerilog list (`--list regress-sv.list`), at the start: 229 of 700 pass (32.7%). The
failures by construct (compile errors): `string` (~60), queues and dynamic arrays `[$]`/`[]`
(~76), `program` blocks (49), file-scope variables (24), classes (19), unpacked dimensions in
typedefs (16), `timeunit`/`timeprecision` (12), plus enum methods (`.name()`, `.next()`),
`$left`/`$right`/`$size`/`$is_signed` and other query functions, `always_comb` rules (31 FAIL:
no PASSED, e.g. always_comb checks).

Next, for real RTL: unpacked dimensions in typedefs, `timeunit`/`timeprecision`, file-scope
variables, `unique`/`priority` case violation warnings, `$countones`/`$onehot`/`$onehot0`
code generation, arrays of interfaces, struct-typed function arguments with patterns; then
testbench features: `string`, queues/dynamic arrays, `program`, classes (large).
