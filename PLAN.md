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
2. **Structural RTL gaps**: multi-dimensional arrays (`reg [7:0] m [0:3][0:15]`), arrays of
   instances (`dff u[7:0] (...)`, port slicing rules), hierarchical references into generate
   blocks (`gen[2].u.sig`), `defparam`, `` `default_nettype none `` enforcement (implicit nets
   are currently always created for port connections), real-valued function arguments in
   expressions everywhere.
3. **Testbench I/O and system functions**: `$fgetc`/`$fgets`/`$fscanf`/`$sscanf`/`$feof`/
   `$fread`/`$fseek`/`$rewind`/`$ungetc`; IEEE `$random` and `$dist_*` (Icarus-identical
   sequences); `$timeformat`/`$printtimescale`; `$dumpvars` scope/level arguments and
   `$dumpall`/`$dumplimit`; `force`/`release` that hold values (also on nets) and procedural
   `assign`/`deassign`; real math (`$sqrt`, `$ln`, `$pow`, `$floor`, ...); `$countdrivers`,
   `$q_*` (rare).
4. **Gate level / netlists**: UDPs (`primitive ... endprimitive`, combinational and sequential
   tables), net types `wand`/`wor`/`tri0`/`tri1`/`supply0`/`supply1`/`trireg`, drive and charge
   strengths (`%v`), `bufif0/1`, `notif0/1`, pull gates, switch primitives (`nmos`, `pmos`,
   `cmos`, `tran`, `rtran`...), gate and net delays (rise/fall/turn-off, min:typ:max),
   inertial delays on `assign`, `specify` blocks (path delays, `$setup`/`$hold`/`$width`/...
   timing checks) and SDF back-annotation.
5. **Tool interfaces and the rest**: VPI/PLI (cocotb and friends), Verilog-2001 configurations
   and libraries (`config`, `-y`/`-v` library search, `` `uselib ``), `$dumpports`, VCD of
   reals and events.

## Testing strategy
- `tests/verilog/*.v`: self-contained testbenches. `tests/difftest.sh` runs each through `iverilog`+`vvp`
  and `ferrooxide` and diffs stdout. `cargo test` runs the same comparison (skipped if iverilog is absent).
- `bench/`: synthetic and realistic designs; `bench/run.sh` reports wall time for both simulators.

## Performance status (CPU cycles, `perf stat`, same machine)

| benchmark | Icarus | ferrooxide JIT | ferrooxide `--no-jit` | speed-up |
|-----------|--------|----------------|-----------------------|----------|
| `bench/cpuish.v` (32 lanes LFSR+ALU, 200k cycles) | 33.6 G | 2.96 G | 5.0 G | **11.3x** |
| `bench/mixed.v` (comb chain, 128-bit datapath, memory, FSM, 300k cycles) | 13.5 G | 0.66 G | 1.4 G | **20.3x** |

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
