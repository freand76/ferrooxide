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
| M2 | fork/join, named events, `disable` across processes, arrays of instances, multi-dim arrays, real numbers, `timescale` + `%t` formatting, `$dumpvars` scopes, automatic/recursive functions | |
| M3 | Nets done right: multiple drivers, resolution (wire/wand/wor/tri0/tri1), strengths (reduced), `assign #d`, inout | |
| M4 | Performance I: levelized comb evaluation, timing wheel, superinstructions, unchecked VM, profiling | |
| M5 | Performance II: Cranelift JIT backend | |
| M6 | SystemVerilog: logic/bit/int types, always_comb/ff, `++/+=`, typedef/enum/struct (packed), packages, interfaces (basic), `$display` extensions | partly (lexing/simple types) |
| M7 | FST output, VPI-lite, better diagnostics | |

## Testing strategy
- `tests/verilog/*.v`: self-contained testbenches. `tests/difftest.sh` runs each through `iverilog`+`vvp`
  and `ferrooxide` and diffs stdout. `cargo test` runs the same comparison (skipped if iverilog is absent).
- `bench/`: synthetic and realistic designs; `bench/run.sh` reports wall time for both simulators.

## Baseline (unoptimized bytecode interpreter)
`bench/cpuish.v` (32 lanes of LFSR + ALU + accumulator, 200k cycles): Icarus 8.7 s, ferrooxide 2.2 s → **4.0x**,
same checksum. 32M process activations, 12.8M NBA updates. Next: M4 (levelized comb + superinstructions).

## Known limitations of the current skeleton
- Nets are treated as variables with a single driver (last writer wins).
- Event expressions must be whole signals (`@(posedge clk)`, not `@(posedge v[3])`).
- Wide (>64 bit) arithmetic uses a slow generic path.
- No real numbers, fork/join runs branches sequentially.
