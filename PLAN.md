# ferrooxide — a fast 4-state, event-driven Verilog simulator in Rust

## Goals
- IEEE 1364-2005 Verilog first, then a pragmatic synthesizable/testbench subset of
  SystemVerilog (1800).
- Full 4-state semantics (0/1/X/Z), IEEE stratified event scheduler (active / inactive / NBA /
  postponed).
- **≥10x Icarus (vvp) throughput** on typical RTL + testbench workloads.
- Output-compatible with Icarus for `$display`-style testbenches: differential testing against
  `iverilog` is the main correctness oracle.

## Architecture

```
 .v/.sv files
     │  ferrooxide-front    preprocessor (`define/`ifdef/`include) → lexer → recursive-descent/Pratt parser → AST
     ▼                      (new front end being written in front/src/syntax, see R1/R2)
   AST
     │  ferrooxide-compile  elaboration: params, hierarchy flattening, port collapsing (aliasing),
     │                width/sign inference (IEEE context-determined rules), codegen
     ▼
  Program  (flat memory image + per-process bytecode + static fanout tables)
     │  ferrooxide-rt       event scheduler + bytecode VM, lowering to fast ops, fusion, Cranelift JIT
     ▼
  simulation
```

**Usage model.** One command, no intermediate files: `ferrooxide tb.v dut.v` parses,
elaborates, compiles to in-memory bytecode (milliseconds) and simulates. Unlike `iverilog` +
`vvp` there is no separate compile step.

**Key design decisions.**
1. *Flat memory*: every signal, temp and constant lives in one `Vec<u64>`. A value of width W
   uses `n = ceil(W/64)` words of `aval` followed by `n` words of `bval` (VPI encoding: 00=0,
   10=1, 01=Z, 11=X). Unused high bits are always zero, so equality is a word compare.
2. *2-state fast path*: every op on ≤64-bit values checks `(b_a | b_b) == 0` once and then
   does plain machine arithmetic. X/Z handling is the slow path.
3. *Static everything*: temps are statically allocated per process (no allocation during
   simulation), event controls get static ids, the signal→waiter fanout is a precomputed flat
   table. Waking a process = one compare + push.
4. *Port collapsing*: ports connected to whole signals (and constant-index array elements)
   alias the parent signal (no copy process).
5. *Processes are coroutines with a single `pc`*: suspension is free.
6. *Rank-ordered combinational settle* (from digsim-native `levels.rs`): every combinational
   process gets a rank above its drivers (Kahn longest path, SCCs/loops excluded); each dirty
   one is evaluated once per settle in rank order, without events in between. Loops fall back
   to events. This gave digsim 45–100x.
7. *Cranelift JIT per process shape*: identical processes of different instances share one
   native function and get their slots/signals/controls from a per-process table.

The two simplifications the design was built around: types are resolved in the parser, and
everything (processes, storage, fan-out) is static. RTL, performance and the debug front ends
fit this; SystemVerilog testbench features and VPI do not (see "Architecture changes ahead").

Also adopted from digsim-native: the value encoding and its `get_bits`/`set_bits` helpers
(`rt/ops.rs`); flip-flops update "in the next round" = our NBA region, edge-triggered processes
grouped per clock edge; deterministic event order so engines can be diffed exactly. Not
(yet) adopted: whole-block "sweep" evaluation when most inputs changed, profiling counters
behind a cargo `prof` feature.

**Scheduling order** mimics Icarus where IEEE leaves it open: wakes are FIFO (sequence
numbers) between active processes and the combinational settle (`tests/verilog/race.v`);
waiters woken by one change run newest first (Icarus's LIFO waiting lists, also for edge
groups, which alternate their member order); fork branches start last to first; time-0
processes start by scope (instances before the instantiating module's own blocks,
`start_order`); `always_comb`/`always_latch` run at time 0 after all other processes have
started (IEEE 1800 9.2.2.2.2).

## Status (2026-11-15)

Regression suite (normal tests): main list 1345 of 1526 (88 %), SystemVerilog list
(`--list regress-sv.list`) 364 of 700 (52 %). Real design benchmark (TetrIC): output identical
to Icarus byte for byte (run, fwd, sound), 10-13x faster. Interactive mode, MCP server,
`--vcd`/`--trace`, the CDSP debug server with value history and the Surfer pairing work.
Per-test regression baseline: `target/regress-baseline/{main,sv}.res`.

| # | Content | Status |
|---|---------|--------|
| M0 | Workspace, plan, CLI, diff-test harness vs iverilog, benchmark | ✅ |
| M1 | Core Verilog: modules/ports (ANSI + non-ANSI), params + overrides, vectors + memories, assign, initial/always, blocking/NBA (incl. `<= #d`), all procedural statements, `#`/`@`/`wait`, all operators with IEEE width & sign rules, functions/tasks, generate, gate primitives, common system tasks | ✅ |
| M1b | VCD: `$dumpfile/$dumpvars/$dumpon/$dumpoff/$dumpall/$dumpflush`, reals, events | ✅ (dumps everything; scope/level args ignored) |
| M2 | `timescale`, reals, plusargs, hierarchical task calls, `$sformat`, `$fopen` modes | ✅ |
| M2b | Testbench concurrency: fork/join (+ join_any/join_none, disable fork), named events, `disable`, automatic tasks/functions, recursion, Icarus wake order | ✅ |
| M3 | Nets: multiple drivers, tri-state, wand/wor/tri0/tri1/supply, static drive strengths, tran islands, inertial delays (rise/fall/turn-off), specify path delays (-gspecify), UDPs, switches | ✅ (strength values on nets / `%v` pending) |
| M4 | Performance I: levelized comb evaluation, fast ops, scratch temporaries | ✅ (timing wheel pending) |
| M5 | Performance II: Cranelift JIT backend, shared per shape | ✅ (wide ops leave the JIT in places) |
| M6 | SystemVerilog | partial, see below |
| M7 | FST, VPI-lite, better diagnostics | |
| M8 | Multi-threading: same-edge / same-rank partitions writing disjoint signals, deterministic | (A5) |
| M9 | CXXRTL Debug Server Protocol (`--cdsp`, `--surfer`) | ✅ first version |
| M10 | MCP server over stdio (`--mcp`) | ✅ first version |
| M11 | Interactive mode (`-i`): run/step/print/list/set/force/release, `restart`, `trace` | ✅ first version |

### Verilog notes
- Fork branches are processes started by `Fork` (last branch first), joins count finished
  branches, `disable` uses a table of named-block code ranges (+ the branches forked inside
  them) and process generations to drop stale wake-ups; events are toggled variables;
  automatic subroutines get storage per call site.
- Recursion: a call of a function that is being inlined becomes a subroutine call
  (`SaveFrame` copies the function's variables and the process's temporaries to a per-process
  frame stack, `CallSub`/`RetSub`; such processes keep no temporaries in JIT registers; depth
  limit 100000 frames). `?:` with calls in its branches evaluates only the selected one.
- Done beyond M1: arrays of instances, `defparam`, `` `default_nettype ``, implicit nets,
  multi-dimensional unpacked arrays and packed vectors (stored flat; index chains become one
  index/select), hierarchical names with genvar indices, file input, `$random`/`$urandom`
  (Icarus sequences), `$timeformat`/`$printtimescale`, `$writemem*`, force/release,
  assign/deassign; static strengths per driver (the strongest wins per bit), pull gates, MOS
  switches, `tran`/`tranif` (islands solved in one process).
- `assign z = a;` (also `{a}`, a constant-index element, `#0`, `wire z = a;`, chains, `logic`
  variables) makes `z` a view of `a` (`alias_assigns`), so it updates immediately as in the
  reference, unless `z` has other drivers, is a special net type, has a strength, a link or
  is forced.
- `$finish` (as in the reference, where it is an event at the end of the queue): the
  processes already scheduled in the time step still run, then its NBAs (woken processes
  do not run) and `$monitor` / `$strobe`; `$stop` stops at once (`Sim::finish_step`).
- Port expressions (`m(a[3:0], .p({b, c}))`, 2026-11-19): each becomes a port net of the
  expression's width and a continuous assignment to the signals (input) or from them
  (output); the signals keep their direction in the AST but are internal (not in the port
  list). A port declared without a range whose net declaration has one is an error (IEEE
  1364-2005 12.3.3), a warning with `-gno-io-range-error` (as the reference).

### SystemVerilog status
Resolved in the lowering (`front/src/lower/types.rs`): `typedef` (module and file scope, unpacked
dimensions: the type's first, ICARUS_NOTES 9, forward typedefs, array typedefs), `enum`
(constants as localparams of the base type; methods, also without parentheses), packed
`struct`/`union` (flat vectors; member/index/select chains become one `[off +: w]` select, also
at run-time indexes and as lvalues), casts `T'(e)`, `N'(e)`, `signed'(e)`, `$bits(T)`,
assignment patterns (positional, and beyond the reference: named and `default:`), packages
with `import p::*` / `import p::x` (module, header, file scope), `pkg::name`, `P::x`/`P::f()`,
`export p::x` / `p::*` / `*::*` (names resolve to the package that declares them),
members and methods of package variables (`P::s.x`, `P::e.next(N)`, `P::c.f()`; read only).
Lint reports names that are SystemVerilog keywords.

Also done: interfaces and modports (interface ports alias the connected instance's scope),
arrays of interfaces and interface port arrays, array query functions, 2-state types
(`bit`/`int`/...: start at 0, stores turn x/z into 0; `UnOp::To2`), `always_comb`/
`always_latch` (sensitivity through called functions, time-0 run), `` `__LINE__ ``,
`$isunknown`, `$countones`/`$onehot`/`$onehot0`, file-scope and package variables (one
instance, shared), `timeunit`/`timeprecision` (module and file scope), time literals,
`input var`/`input logic` port kinds, `-u` (separate compilation units), `unique`/`priority`
case and if warnings for unmatched values, `break;`/`continue;`, prefix `++a;`/`--a;`,
`inside` and `case ... inside`, `foreach`, several variables and steps in a `for` header,
`iff`, `.triggered`, `wait fork`, `program` blocks (also nested in modules; no `always`, instances,
gates or nested declarations in them), nested module declarations (23.4: without ports
instantiated once under their own name; names not found inside are looked up in the
enclosing instance), the checks on `final` procedures (no NBAs, delays, event or wait
controls, task calls), immediate `assert`/`assume` (default `$error`; `cover` does nothing,
as in the reference), `$error`/`$warning`/`$info`/`$fatal` with file, line, time and scope,
initial values of variables in functions and tasks (static: once; automatic: per call;
constant calls start afresh), `module automatic`, whole unpacked array assignments (continuous: an alias for the same shape, else per
element; procedural: per element) and array ports (an alias of an array of the same shape),
`void'(f())`, `->`/`<->`, enum name ranges (`N[3]`, `M[5:3]`), `$countbits`, array
queries of parameters, expressions and queues, `wire integer`/`wire time`, `++`/`--` inside
expressions (with short-circuit `&&`/`||`/`?:` when the
right side has side effects).

*A1, first part (types in elaboration).* Packages and `$unit` are scopes of their own,
elaborated once (`Module::package`; `Scope::imports` for lookups after a module's own names).
Their variables are shared, their functions and tasks run in their scope (its names and time
unit); a module finds package and `$unit` names (parameters, enum constants, functions)
through `Scope::imports`, and constants in types declared there refer to that scope
(`Lower::qualify_type`). Type values in
the compiler: `Sym::Type` / `PVal::Type` hold an `SvType` with its dimensions evaluated
(`resolve_type`), `Expr::Type` carries a type in a parameter list. `parameter type` works for
declarations, ports, `$bits` and casts, per instance. Left: struct layout and member access
are still resolved in the parser (`struct_access`), so variables of a type parameter have no
members and unpacked structs are not possible; the compiler still types expressions as
(width, signed, real).

*A2, first part (strings).* A string value is the 64-bit handle of an interned text
(`Program::strings` from the compiler, the table in `Sim`), so equal texts have equal handles
and stores, change events and `==` need nothing new; the rest are `Instr::Str` operations on
the generic path (`compile/src/strings.rs`, `Sim::exec_str`). The compiler knows a string
signal by `SignalInfo::string` and types expressions with `is_string` next to `is_real`
(`strings::VTy` for assignment targets). Interning never frees a text.

### Performance

CPU cycles (`perf stat`, same machine):

| benchmark | Icarus | ferrooxide JIT | ferrooxide `--no-jit` | speed-up |
|-----------|--------|----------------|-----------------------|----------|
| `bench/cpuish.v` (32 lanes LFSR+ALU, 200k cycles) | 33.6 G | 2.96 G | 5.0 G | **11.3x** |
| `bench/mixed.v` (comb chain, 128-bit datapath, memory, FSM, 300k cycles) | 13.5 G | 0.66 G | 1.4 G | **20.3x** |

With a full VCD dump (all signals; wall time, same machine, 2026-11-13):

| benchmark | Icarus | Icarus + `$dumpvars` | ferrooxide | ferrooxide `--vcd` | VCD size |
|-----------|--------|----------------------|------------|--------------------|----------|
| `bench/cpuish.v` | 14.3 s | 19.5 s | 0.73 s | 1.95 s | 707 MB (Icarus 720 MB) |
| `bench/mixed.v` | 3.1 s | 7.4 s | 0.15 s | 0.51 s | 175 MB (Icarus 308 MB) |

Dumping everything costs ferrooxide 2.7-3.4x (mostly formatting and writing the text); the
lead over Icarus with a dump is 10-14x. `--trace REGEX` limits the cost to the selected
signals. Real design (TetrIC run target): about 25-32 s wall with the machine loaded; compare
builds with `perf stat -e instructions` (231 G at 69f7fb4).

What got us here (in order of impact):
1. Lowering to single-word fast ops with the 2-state check inline (`rt/fast.rs`).
2. JIT code shared per process shape: without sharing, 32 lane copies thrashed the
   instruction cache (586M i-cache misses → 2M).
3. Shared scratch region for temporaries (dead at every suspension point): L1d misses
   161M → 42M.
4. Rank-ordered combinational settle as a dirty bitset in rank order, with a separate
   combinational fan-out list that needs no per-process state.
5. Constant-index array accesses resolved at compile time; inline `Wait` and inline NBA queue
   appends in JIT code.
6. Element-level sensitivity for constant-index array accesses (virtual element signals,
   element-granular rank graph): mixed.v 27.6M → 8.4M activations.
7. Wide (≤1024-bit) ops compiled natively (unrolled word loops), concatenation as bit-field
   inserts, scratch temporaries promoted to SSA variables unless a generic callback reads them.
8. Ports connected to constant-index array elements alias the element.
9. Fusion of combinational chains (`rt/fuse.rs`): a pure combinational process whose only
   combinational consumer is B is merged into B (cpuish 26.0M → 19.6M activations, mixed
   3.6M → 1.2M).
10. Fast paths for variable-index memory reads/writes, part-select stores, signed compares,
    logical not and wire resolution; edge groups (clocked processes with identical triggers
    run as one activation).

Profile (2026-11-14, `perf record -g`; cpuish 2.9-3.0 G cycles / 7.96 G instructions, mixed
0.68 G / 1.82 G):

| function | cpuish | mixed |
|----------|--------|-------|
| `Sim::notify` (`rt/vm.rs`) | 36 % | 25 % |
| JIT code (all shapes) | 39 % | 25 % |
| `Sim::advance_steps` (scheduler loop) | 12 % | 19 % |
| `Sim::exec` (generic instructions) | 10 % | 7 % |
| future queue: `BTreeMap` entry + malloc/free/realloc of `Vec<Fut>` | 1.5 % | 7.5 % |

Measuring: cycles move by up to ±4 % between builds whose instruction counts are identical
(code layout of `notify` / `advance_steps` after unrelated changes: cpuish 2.86 G at 7d1ee50,
2.96 G after the SystemVerilog work, both 7.964 G instructions). Compare
`-e instructions,cycles`, several runs each; a layout-only swing is not a regression, and
pinning the hot functions' alignment (or PGO) may recover it.

### Debug front ends (M9, M10, M11)

**CDSP** (spec https://cxxrtl.org/protocol.html, v0.14.0, protocol version 0, still
changing). NUL-terminated UTF-8 JSON over TCP or a Unix socket. Commands: `list_scopes`,
`list_items` (direct children; ids are space-separated hierarchical names, root `""`),
`reference_items`, `query_interval` (samples and diagnostics for an inclusive interval,
optionally collapsed), `get_simulation_status`, `run_simulation` (`until_time`,
`until_diagnostics`), `pause_simulation`; events `simulation_paused`, `simulation_finished`.
Times are `"<s>.<fs>"` (ticks × 10^(15 + precision) fs); values per item are little-endian
u32 words in base64; no value setting in the spec yet.

Implementation (`cdsp.rs`): listener thread, one reader thread per connection; the simulation
thread owns `Sim` and runs `advance_steps` chunks of 10-40 ms, checking the command channel
between time steps, so the hot loop needs no locking. History (`rt/history.rs`): one
sequential log (signal id + planes packed to the width, 6 bytes per 1-bit change) fed from the
dirty list at the end of each time step (`Sim::history_start`; `track` replaces `dumping` as
the dirty-list switch), budget `FX_HISTORY_MB`; per-signal indexes are built on demand.
Samples are sent at the interval start and at each change of a referenced item. Reals are
64-bit items (IEEE double bits). `base64(u32)` is two-state: x/z are sent as the value plane
(x→1, z→0). Cost on `bench/cpuish.v` (all signals toggle): 0.64 s plain, 1.1-1.3 s with full
history (290 MB); a query of 10 signals over the whole run (400k samples) about 0.85 s, the
first one 0.45 s more (indexing).

**Surfer as client** (its source, 2026-11-03 snapshot, `libsurfer/src/cxxrtl*`;
`surfer cxxrtl+tcp://HOST:PORT`, TCP only in native builds). Its greeting check needs
`commands`, `events`, `features` (contents ignored). `list_scopes {scope: null}` expects *all*
scopes in one flat map; `list_items` (with `null` and per scope) reads only `width`; every item
is taken as a node, so a memory item would make its whole reference fail. `reference_items
"ALL_VARIABLES"` is re-sent whenever a signal is added; `query_interval` always asks for
`[0, latest_time]`, `collapse: true`, no diagnostics, after every status change (no
incremental fetch; it looks up the latest sample <= t). Play = `run_simulation until_time =
latest + 100 ns` (hard-coded); `pause_simulation` expects `{time}`; no polling while running.
Responses carry no ids and are paired FIFO (an error pops one too): answer strictly in order.
Values are decoded as unsigned integers: x/z are lost.

**WCP** (Surfer's Waveform Control Protocol, `surfer-wcp`, unfinished, NUL-terminated JSON):
`add_items`, `set_cursor`, `set_viewport_to/range`, `add_markers`, `load`, `reload`, `clear`,
`zoom_to_fit`, `set_item_color`, `get_item_list/info`, `remove_items`, `focus_item`; events
`waveforms_loaded` and, if listed in the tool's greeting, `goto_declaration`, `add_drivers`,
`add_loads`. `surfer --wcp-initiate PORT` connects to a listening tool (127.0.0.1). `load`
accepts only files and URLs, and `waveforms_loaded` is only sent for file sources.

**Pairing (`--surfer`).** ferrooxide opens a CDSP and a WCP listener and starts
`surfer cxxrtl+tcp://127.0.0.1:<cdsp> --wcp-initiate <wcp>`; after the WCP greeting it sends
`add_items` (dotted names), retried every 250 ms while Surfer answers "No waveform loaded";
`$finish` gets a marker (WCP times are fs for a CXXRTL source); ferrooxide exits with Surfer.
Without `-i` it runs at once and pauses after 2 s of wall time if not ended (Surfer queries
only at status changes); without `--trace` the top modules' signals are shown (`--trace-all`:
all, at most 2000); the first values after time 0 get one `zoom_to_fit`. Console (`-i`):
cursor follows run/step/breakpoints, `restart` reloads the design (sources and data files as
they are now) and starts at time 0 on the same connections (viewer gets `simulation_paused` at
0; a running VCD trace is written anew), `quit` closes the viewer; line editing and history
(`~/.ferrooxide_history`, rustyline). Checked with Surfer 0.7.0 (2026-11-14, logging proxy on
both connections): greeting, scopes, items, `reference_items`, `query_interval` and
`add_items` work.

Surfer 0.7.0 (11bb2e3) shows no live waves because of two bugs in its CXXRTL path (fixed
locally in `/projects/surfer`, to report upstream): (1) `WaveData::cached_time_range` is
computed at load (latest time 0 → 1 fs) and never refreshed for a live source: refresh it on
`Message::InvalidateDrawCommands`; (2) `variable_draw_commands` skips every variable whose
`signal_id` fails, and `WaveContainer::signal_id` is not implemented for CXXRTL: skip the check
for CXXRTL. With both fixes the waves show. Its value cache never forgets samples, so after a
`restart` it can show the previous run's samples (feature request: clear the cache when the
latest time goes backwards).

**MCP** (`--mcp`, JSON-RPC 2.0 over stdio; https://modelcontextprotocol.io). Lets an agent
load, run, inspect, force and lint a design instead of reading logs and huge VCDs. It shares
the core with CDSP and `-i`: `Sim::start/advance/finish`, signals read, deposited, forced and
released by name, the hierarchy model, the change-log history. Tool set of the first cut:
`load_design {files, defines, include_dirs, top, plusargs, dialect}` (structured errors with
file:line), `lint`, `run {until_time | for_time | until_condition, max_steps}` (stop reason,
time, `$display` output truncated with a count), `list_scopes`/`list_signals {scope,
pattern}`, `get_values {signals, radix}` (x/z as in `$display`), `get_history {signals, from,
to, max_changes}`, `set_value`/`force`/`release`, `add_breakpoint {condition}`/`list`/
`remove_breakpoint`, `dump_vcd {file, scopes, from, to}`, `reset`/`restart`, `status`.
Resources: source files, elaborated hierarchy, simulation log. Rules: every result small and
bounded (truncation with counts, pattern filters), structured errors with locations,
deterministic, long runs cancellable, a memory budget for history.

## Testing
- `tests/verilog/*.v`: self-contained testbenches; `tests/difftest.sh` runs each through
  `iverilog`+`vvp` and `ferrooxide` and diffs stdout. `cargo test` runs the same comparison
  (skipped if iverilog is absent).
- `tests/suite/<area>/*.v` (`tests/suite.sh`, also in `cargo test`): our own suite, no other
  simulator needed, every test in all three modes. Self-checking (print `PASSED`, never a line
  starting `FAILED`) or compared with a checked-in `<name>.expected` (`// expected`; `--bless`
  writes it: review it). Expected behaviour comes from IEEE 1364/1800 (cite the section);
  where the reference agrees, that checks the test itself. Order IEEE leaves open (wakes of
  one change, races) belongs in `tests/verilog`. Areas: `sched`, `sv`, `expr`, `nets`,
  `systasks`, `hier`, `subr`, `real`, `vcd` (reads its own dump back), `prims`, `arrays`,
  `directives`, `events`. Next areas: `$sformatf` corner cases, `` `timescale `` inheritance
  across files, `disable` corner cases; ideas from the regression suite's coverage, tests
  written fresh.
- Pitfalls: fork branches start last to first (here and in the reference), so a trigger in one
  branch for a waiter in another needs a `#0`. A task argument is context-determined by the
  task's input width (`check(x, a + b)` with a 64-bit input widens `a + b`); wrap
  self-determined expressions in `{...}` and route signed values through `integer`.
- Parser snapshots: `front/tests/parse.rs`, `tests/parse/*.sv` → checked-in `.ast`
  (`--dump-ast`, `front/src/dump.rs`).
- Regression suite: `tests/ivtest.py` (`--show 40` groups failures), per-test comparison
  against the baseline with `tests/regress.sh`. Also chipsalliance *sv-tests* for parser
  coverage.
- `bench/`: synthetic and realistic designs; `bench/run.sh` reports wall time for both
  simulators.

## Next steps (in priority order)

1. **Restructuring** (R1-R6 below, R0 done). The new front end is being written now.
2. **Types in elaboration** (rest of A1, = R4): a type per signal in the compiler (string,
   queue, ...) and expression typing that uses it; then struct layout and member access move
   out of the parser.
3. **SystemVerilog RTL items**: struct-typed function arguments with patterns (`export` in
   packages done 2026-11-18).
4. **Heap objects** (A2): queues, dynamic and associative arrays (mutable objects: handles to
   a separate heap with copy on assignment).
5. **Dynamic processes, subroutine calls, dynamic waiters** (A3). *First part done
   2026-11-19*: a fork branch started again while it still runs (fork/join_none in a loop)
   becomes another instance of the branch: a process id from `Sim::inst_base` on, running
   the branch's code (JIT: a copy of its table with the instance's id), woken through the
   branch's fan-out entries (`notify_insts`). The branch's private memory
   (`ProcDef::frame`: temporaries that live across waits, frames of automatic subroutines it
   calls, automatic block and loop variables) is swapped between instances when another one
   runs. Automatic variables of a fork block are part of each branch's frame and keep the
   value the forking process gave them (`ProcDef::capture`, `Instr::Evict` before the
   block sets them). Cost: about 3% more cycles on bench/cpuish (instance checks in
   `exec`/`notify`). Left: the branches of one fork each get their own copy of the fork
   block's automatic variables (the standard shares one between them), real subroutine
   calls, dynamic waiters for run-time targets. Then classes (programs done 2026-11-18); VPI (A4) on the same base. (First SV measurement, as a
   size hint: queues/dynamic arrays ~76 failing tests, `program` 49, classes 19.)
6. **Remaining regression failures** (`tests/ivtest.py ... --show 40` groups them): the
   Verilog items under "Unsupported (known)", from compiler warnings and `%v` (needs strength
   values on nets) to force on array elements.
7. **Debug front ends**: report the Surfer bugs upstream; MCP over streamable HTTP (attach to
   a running simulation), MCP history queries on the CDSP value history, MCP resources; CDSP
   arrays (per-element items for small memories; memories with `depth`/`zero_at`), port
   directions (`input`/`output` are false; needs direction in `SignalInfo`), `src` from
   declaration locations, diagnostics (`$display`/`$write` → `print`, `$finish`/`$stop` →
   `break`, assertions → `assert`; kept in the history, `until_diagnostics`); WCP pushes:
   `--trace REGEX` signals via `add_items`, `set_cursor` at breakpoints and `$stop`,
   `$display`/assertions as markers, `goto_declaration` from declaration locations,
   `add_drivers`/`add_loads` from the elaborated netlist. Interactive mode: watch, conditional
   breakpoints, waveform history, scopes.
8. **Performance** (in parallel, when speed matters again):
   - `notify` (largest item): per change it follows `parent` (array elements), checks the
     dirty list, sets one comb bit per combinational reader with a `comb_wake` call each, then
     scans the event fan-out (edge check from the value's LSB) and sorts `wake_buf` when more
     than one waiter wakes. Ideas: per-signal flags precomputed at build time (no event
     fan-out / only combinational readers / no parent) so the common case is one bit loop;
     set comb bits of small fan-outs directly in JIT code; skip the sort when waiters are
     already in `wseq` order.
   - Timing wheel for `Sim::future`, and reuse the `Vec<Fut>` of a finished time slot instead
     of allocating one per slot (`BTreeMap::entry().or_default()`).
   - Wide (>64-bit) ops that still leave the JIT (`FX_GENSTATS=1` counts them).
   - Nets with several drivers resolved inline at the driver's store (as the reference does)
     instead of by a process in the settle; saves activations.
   - `advance_steps` per-step bookkeeping (dirty list, postponed, NBA swap): `perf annotate`
     before changing it.
   - Ports connected to part selects as aliases, fusing sibling processes with the same
     inputs, signal values cached in registers within an activation, an optional 2-state
     shadow for signals proven never to hold X.
   - Multi-threading (M8, A5) last.
9. **Outputs**: FST, VCD of arrays, `$dumpvars` scope/level arguments, `$dumplimit`,
   `$dumpports`.
10. **Verilog-2005 rest**: VPI/PLI, configurations and libraries (`config`, `-y`/`-v`,
    `` `uselib ``), timing checks, SDF back-annotation, see Unsupported.

### Restructuring for maintainability (review 2026-11-15)

Adding SystemVerilog has become slow because of how the front end is built:
- the parser does semantics: `svtypes.rs` resolves types, struct layouts, patterns, enum
  methods and array queries while parsing, so `Parser` keeps a symbol table (24 fields:
  `vars`, `types`, `packages`, `unit_*`, `decl_type`, `last_type`, `ret_type`, ...);
  whole-scope knowledge needs workarounds (`early_typedefs`, `$root.` stripped from the
  tokens, `scoped_copy`/`package_items` copying package items into every module);
- `parse_files` moves ~15 parser fields from file to file by hand;
- the AST has Verilog-2005 shapes only: SV is encoded as pseudo calls (`$__cast`,
  `$__pattern`, `$__str`, `$__named`, `$__tostring`, ...), `Expr::Hier` means a hierarchical
  name, a struct member, a method call or `.triggered`; types exist three times (`PType`,
  `SvType`, the flags on `Decl`/`ParamDecl`);
- keywords are strings (`Tok::Ident("module")`, `KEYWORDS`/`SV_ONLY`/`SV_RESERVED`);
- `compile/lib.rs` is 3500 lines, `Compiler` ~70 fields across elaboration, nets, codegen,
  strings and lint; an expression's type comes from `ty`, `is_real`, `is_string`, `lvalue_ty`;
  `Expr` has no position (errors use `self.line`).

Steps (each ends with all tests in all three modes and unchanged regression counts):
- **R0. Safety net.** *Done 2026-11-15*: `--dump-ast` and parser snapshot tests (see Testing),
  per-test regression baseline.
- **R1/R2. A new, complete front end beside the old one** (plan 2026-11-15). Goal: the parser
  accepts all of IEEE 1800-2017 (Annex A) even where elaboration has nothing behind it yet;
  unsupported constructs fail in the lowering with "X is not supported" at their line, never
  silently.
  - *F1. Tokens* (`front/src/syntax/{kw,token,lexer}.rs`): a `Kw` enum for every 1364-2005 and
    1800-2017 keyword, line and column per token, the SV operators the old lexer splits (`##`,
    `|->`, `|=>`, `<->`, `->>`, `:=`, `:/`, `@@`, `&&&`, `#-#`, `#=#`, `'{`, ...). Keywords
    added by 1800 are "soft": accepted as names where only a name can stand (what the old
    parser does with `SV_ONLY`/`sv_ident_here`). Dialect-dependent keyword sets
    (`begin_keywords`, `-g2005`) come after the switch, as their own step with a regression
    check.
  - *F2. Syntax tree* (`syntax/tree.rs`): what the source says, nothing resolved. Source
    units with descriptions (module/interface/program/checker, primitive, package, class,
    config, bind, file-scope items, directives in source order); a syntactic `DataType`
    (integer/real keywords, struct/union, enum, named `pkg::T` / `C#(..)::T`, virtual
    interface, `type(e)`, packed/unpacked/queue/dynamic/associative dimensions); declarations
    as written (net/var/port/param groups, ANSI port lists without inheritance applied);
    expressions with members, scoped names, calls with named arguments and `with`, casts,
    patterns, streaming, `inside`/`dist`, `new`, tagged; all statements (incl. randcase,
    randsequence, assertions, `wait fork`, `->>`, clocking drives); properties and sequences;
    classes, constraints, covergroups, clocking blocks, specify (paths and timing checks), UDP
    tables, DPI, `let`, nettypes.
  - *F3. Parser* (`syntax/parse/*.rs`, one file per area): recursive descent, no semantics.
    Only state: scoped sets of type names (typedefs, type parameters, classes, forward
    typedefs) and the type names each package exports, to parse `T x;`, `T'(e)`, `$bits(T)`.
    `--parse-only`, `--dump-syntax`. Corpus check `tests/parsecheck.py`: every sv-tests file
    (`~/projects/sv-tests`, 1028; those marked to fail in parsing must fail), every regression
    suite source, `tests/`, and the local real designs; per-file results against a stored
    baseline, as for the regression suite.
  - *F1-F3 status (2026-11-15)*: done, parse only (`--parse-only`, `--dump-syntax`). The
    tokens come from the old lexer (identical numbers and strings), converted in
    `syntax/token.rs`. Every file the old parser accepts parses; sv-tests 896 of 914 that
    must parse (the rest are preprocessor gaps: `` `undefineall``, predefined `` `SV_COV_*``,
    include names from macros; 13 directive tests that should fail do not); 92 need UVM.
    Baseline in `target/parsecheck-baseline.res`.
  - *F4. Lowering* (`front/src/lower/`): syntax tree → today's AST, so the compiler does not
    change. It holds what `parser.rs`/`svtypes.rs` do besides parsing (type and variable
    tables, struct member chains, patterns, enum constants and methods, array queries,
    packages and `$unit`, time literals, foreach/case-inside/unique/iff/`++` expansions,
    `name[@N]` segments, unnamed instances), walking the tree in source order. `FX_FRONT=new`
    selects it; `tests/frontdiff.py` compares `--dump-ast` of both front ends over the corpus.
  - *F4 status (2026-11-15)*: done (`front/src/lower/`). `--dump-ast` equal for all own
    tests, 3332 regression-suite files and 315 real-design files; the differences are fixes:
    `int unsigned` functions/parameters were signed, `$bits(logic [3:0])` was a select of a
    variable `logic`, an ANSI UDP output initializer (`output reg q = 0`) was dropped, and
    the new front end accepts what the old one could not parse (`W'(e)` size casts,
    `byte`/`shortint`/`longint` parameters and return types, `name: begin`, ...; regression
    suite: 23 SV tests from compile error to pass, none worse). Rejected as before:
    explicit variable lifetimes in subroutines and `module automatic` (the compiler does not
    apply initial values of subroutine variables yet). `expect(e);` without an action is a
    call of a task named `expect`; a bare `new` is a name when a variable `new` exists.
  - *F5. Switch*: when `frontdiff` is equal (or each difference is understood and better),
    `cargo test` passes in all modes and `tests/regress.sh` shows no test worse, the new front
    end becomes the default; then `parser.rs` and the parsing half of `svtypes.rs` are deleted
    (type math moves to `lower/types.rs`), `FX_FRONT` goes.
  - *F5 status (2026-11-15)*: done. The new front end is the only one; `parser.rs`,
    `svtypes.rs` and `tests/frontdiff.py` are gone (`dec` is in `ast.rs`, the `name[@N]`
    index table in `lower/mod.rs`). Regression suite as at F4 (23 better than the old
    front end, none worse).
  - *F6.* CLAUDE.md rules: the parser resolves no names other than type names; new syntax gets
    a tree node and a parser snapshot, never a `$__` pseudo call; the lowering reports what it
    cannot do.
  - *F6 status (2026-11-15)*: done (CLAUDE.md, "Front end").
- **R3. Compilation units.** (F4 replaces the field shuffle in `parse_files`.) Package items
  are no longer copied into modules (package scopes resolve them).
  *Done 2026-11-15*: modules, packages and `$unit` keep only their own items; names come
  from the imported scopes (`param_range` too). Left: a named import (`import p::X`) and a
  reference `p::Y` make all of `p` visible, not only the imported name (as before).
- **R4. Types in the compiler** (rest of A1): a `Type` per signal, one
  `type_of(expr) -> ExprType` replacing `ty`/`is_real`/`is_string`/`lvalue_ty`; members and
  patterns resolved in elaboration (type-parameter structs, unpacked structs); packed types
  still flattened at the end (runtime/JIT unchanged); the compiler reads the syntax tree's
  types, and the lowering shrinks piece by piece until the old AST is gone.
  Slices (each ends with all tests in all three modes and unchanged regression counts):
  1. A `Type` per signal (vector, real, string, packed struct), filled from today's
     declarations, and one `type_of(expr)` replacing `ty`/`is_real`/`is_string`/`lvalue_ty`;
     no behaviour change. `elab_items` (`elab/items.rs`) split per item kind where it is
     touched.
     *Done 2026-11-18*: `types.rs`: `Type` (vector, real, string) per signal in
     `sigs.types` (every signal goes through `push_signal`; `mark_real`/`mark_string` set
     it), `type_of(expr)` computes it bottom-up and replaces `ty`, `is_real`, `is_string`,
     `lvalue_ty`, `lvalue_is_real`, `sig_vty` and `VTy`; the compiler no longer reads the
     runtime's `real`/`string` flags. Packed structs are not a `Type` yet: declarations
     still arrive flattened from the lowering, so they come with slice 2.
  2. Struct member access and assignment patterns resolved in elaboration from the `Type`;
     the lowering's `struct_access` and pattern code go; members of variables of a type
     parameter work.
     *Done 2026-11-18*: declarations, parameters and function results keep their data type
     (`Decl::ty`, `ParamDecl::ty`, `SubroutineDecl::ret_ty`); the AST has `Expr::Member` and
     `Expr::Pattern`. The compiler gives each typed signal a `Packed` layout
     (`sigs.layout`; struct parameters `$type:p`), turns member chains into a part select
     of the flat storage in `md_norm` (as for several packed dimensions) and resolves
     patterns against the target's layout (assignments, declarations, parameters, `return`,
     function arguments, `T'{...}`). New: members and patterns of type-parameter variables,
     patterns as function arguments. Left: members through hierarchical names
     (`inst.v.f`); patterns for unpacked arrays.
  3. Unpacked structs (and structs with reals or strings).
     *Done 2026-11-18*: an unpacked struct is stored flat like a packed one, but its members
     may be `real` (f64 bits) or `string` (a handle) at their offsets: they start as 0.0
     and "", read and write as real / string (`member_type`; patterns convert per member).
     A packed struct with a non-integral or unpacked-struct member is an error (7.2.1).
     Left: unpacked array members, member defaults, method calls on members
     (`s.name.len()`), `%p` of a whole struct.
  4. Then A2 (queues, dynamic and associative arrays) builds on the same `Type`.
     *4a done 2026-11-18*: queues and dynamic arrays. A variable holds a 64-bit handle of
     an object on the simulator's heap (`Instr::Heap`, generic path; 0 is the empty array);
     each variable owns its object (an assignment of another variable's array copies it,
     the old one is freed on blocking assignments), elements and methods change it in place
     and give the variable a change event (`Touch`). `Type::Dyn`, `compile/heap.rs`; front end: `Declarator::heap`,
     `SvType::heap` (typedefs, return types), `Expr::NewArray`, `Expr::Last` (`$`), `{}`,
     foreach.
     *4b done 2026-11-18*: associative arrays (`[string]`, integer keys up to 64 bits, `[*]`):
     a map on the same heap (`Instr::Assoc`), num/size/exists/delete/first/last/next/prev,
     missing keys read as the default, copies, foreach in key order (`first`/`next`).
     Left: keyed patterns (`'{"a": 1, default: 0}`), class or wider keys.
     *4c done 2026-11-18*: bounded queues (`[$:N]`: at most N + 1 elements; pushes and
     inserts into a full one drop the last element or are skipped, assignments keep the
     first elements) and the reference's run-time warnings of queue operations, with the
     source location under `-pfileline=1` (`Program::locs`; `tests/ivtest.py` passes the
     option). Left from 4a: queues of structs, arrays of queues,
     queue ports, `%p`, array methods (`sort`, `find`, ... `with`), element writes with
     `<=` (done at once).
- **R5. Split the compiler**: `elab/{params,hier,decls,generate,nets,udp,specify}.rs` plus
  codegen; `Compiler` fields grouped into sub-structs. Move first, regroup after.
  *Done 2026-11-15*: `elab/{hier,params,decls,items,nets,udp,specify}.rs` and `codegen.rs`
  (the item walk `elab_items` is `elab/items.rs`, 630 lines, still one function: splitting
  it per item kind can come with R4); `lib.rs` keeps the state, scopes, lookup and constant
  evaluation. Fields in `hier`, `diag`, `code`, `subr`, `nets`, `sigs`; `prog`, `scopes`,
  `file`, `line`, `gprec`, `dialect` stay at the top.
- **R6. Spans** (file, line, column) on `Expr`/`Stmt` instead of `self.line`: part of R4,
  not a step of its own. The syntax tree already has a `Loc` on every expression and
  statement (a column is a small parser change, tokens carry positions); spans come with
  the compiler reading the tree instead of the old AST, so the old AST gets none.

### Architecture changes ahead (review 2026-11-14)

**Fits as is.** Wide JIT ops, timing wheel (`future` is a `BTreeMap` behind a few calls),
part-select port aliases, strength values (per-bit strength data next to the value, only on
nets with resolve processes / tran islands; the value encoding stays), FST, `$dumpvars`
arguments, CDSP/MCP history and breakpoints between time steps, the remaining Verilog-2005
gaps.

**A1. Types in elaboration** (= R4; first part done, see SystemVerilog status). The compiler
knows an expression only as (slot, width, signed) plus `is_real`/`is_string`, which cannot
express unpacked structs (or structs with reals / strings), queues, dynamic/associative arrays
or class handles, all with their own operator rules. A `Type` enum in the compiler (vector,
real, string, queue(T), darray(T), assoc(K,T), class(id), unpacked struct, ...) carries the
meaning. Prerequisite for most of the remaining SystemVerilog.

**A2. Heap objects.** Strings, queues, dynamic and associative arrays live in a heap of
objects, referred to by handles stored in the flat memory (one value word; a null handle is
0). Ops on them are generic-path instructions at first. No dynamic processes needed.

**A3. Dynamic processes, calls and waiters.** Today a process is a fixed `ProcDef` with one
`ProcRt`, fork branches are fixed processes that get restarted (hence fork/join_none in a
loop), automatic storage is per call site, functions are inlined (recursion copies frames),
and fan-out is a fixed range of `(proc, ctl)` entries. Classes, fork in a loop, VPI callbacks
and MCP conditional breakpoints all need:
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
only for processes without system tasks. Pays off only on large designs.

**CDSP later.** Phase 2 history (CXXRTL's approach): periodic checkpoints of the whole state
(`mem`, process states, queues, NBA buffer, RNG; file handles muted) and deterministic
re-simulation from the nearest checkpoint. Setting values (when the spec adds it: force-like
writes through the force mask), breakpoints on signal conditions, scope/item attributes
(`(* ... *)` are dropped by the lexer today), a VCD/FST import mode as a read-only server.
Upstream proposals: a four-state value encoding (value + unknown planes, e.g. a
`base64(u32,4state)` feature) in CDSP and in Surfer's decoder; incremental `query_interval`;
a configurable run step instead of the fixed 100 ns.

**Open questions.** How large a CDSP history is useful by default (it is re-sent at every
pause), and whether to expose x/z before the spec defines it. MCP: one server per design or
several sessions; breakpoint conditions likely compiled by the normal expression compiler to a
small process that wakes on the signals; how to present large memories.

**Watch.** Elaboration and codegen run per instance (only the JIT shares code between
instances); measure compile time and memory on a large netlist before assuming it scales.

## Differences from the reference

Orders IEEE leaves open and other deliberate or known differences (Icarus deviations from the
standard that we match are in `ICARUS_NOTES.md`):
- On a change, a ranked combinational process (`always_comb`, `always @*`) runs before the
  other waiters of the signal; the reference wakes it among them (newest waiter first).
- The reference propagates some net updates immediately, inside the writing process
  (`a = 1; $display(z)` shows the new value). Here only for `assign z = a`-style aliases (see
  Verilog notes); nets with several drivers, `wand`/`tri1`, strength drivers and
  `assign z = m[i]` with a run-time index update in the combinational settle (immediately
  there). Not immediate there either: logic (`a & b`), part selects, widening.
- A genvar in a port expression is sized differently (`8'd10 + g` to an 8-bit port: "given 5
  bits, padding" there, "given 32, pruning" here).
- The VCD is not byte-identical: the reference orders variables by kind (events, nets, regs,
  integers, reals), gives a port and the net it is connected to separate codes and names nets
  `wire`; here one code, and an output port collapsed onto a variable is `reg`.
- Beyond the reference: named and `default:` assignment patterns. Like the reference,
  overlapping `unique` case/if items are not reported.
- At time 0 the reference lets `always @(n)` see the first value of a net driven by an
  operator (`assign n = d <<< 2;`: an x event) but not of one driven through a part select;
  here continuous assignments settle before other processes wait [pr1002a].
- Two processes woken by delays scheduled in the same time step and then waiting on the
  same edge (tasks called hierarchically) run in another order than in the reference
  [pr2001162].

## Unsupported (known)

Kept up to date as features stop short; regression-suite test names in brackets.

SystemVerilog data types and testbench features:
- queues of structs, arrays of queues, queue ports, `%p`, array methods (`sort`, `find`, ...
  `with`), queue element writes with `<=` (done at once), keyed patterns for associative
  arrays, class or wider keys (see R4 slice 4).
- classes (first part done 2026-11-18: properties, constructors, methods inlined with
  `this`, `extends`/`super`, virtual dispatch, file-scope and package classes, static
  properties (2026-11-19: a variable `C$p` beside the class), parameters, typedefs and
  enums in classes (constants in the class's scope, `C::P`, `obj.P`, `C::T`); objects never
  freed). Typed constructor calls `C::new(args)` (2026-11-19). Left: static methods, `C::f()`, enum methods on an enum property from
  outside the class, parameterized classes, forward `typedef class`, arrays of handles
  and array properties, class handle ports, string methods on properties, NBAs to
  properties, `void'(obj.f())`, pure/extern methods, randomize, interface classes.
- programs: processes run in the Active region, not the Reactive one (as the reference,
  which warns about assignments to module variables), no implicit `$finish` when all
  program `initial` blocks end, no `program` limits on what a module may reference.
- `static` variables in blocks inside functions and tasks, and a variable lifetime other
  than the subroutine's (`static` in an automatic function) [func_init_var2,
  task_init_var2].
- struct members of variables whose type is a type parameter; unpacked structs (struct
  layout is still resolved in the parser, A1); struct-typed function arguments with patterns.
- nested modules with ports instantiated from outside their module; multi-dimensional
  interface port arrays; modport directions are not enforced (writing an input of a modport, members not
  listed in it) [sv_interface_port_*restrict_fail, *_write_fail, *unlisted_member_fail];
  a forward typedef defined in another scope than its declaration.
- assignments to members or selects of package variables by their scoped name
  (`P::s.x = 1`, `P::v[2] = 1`).
- `.triggered` only on events declared in a module (not in tasks, functions or blocks).
- `iff` only when every item of an event control has the same condition, and only in an
  event control before a statement (not in intra-assignment controls).
- nets of 2-state types (`wire bit`, `wire int`; a reference extension, 6.7.1 allows only
  4-state types) [implicit_cast4-13, br_gh337].
- deferred (`assert #0`, `assert final`) and concurrent assertions, properties, sequences
  [sv_deferred_assert*]; a compound assignment or increment whose target index has an
  increment (`x[i++] += 2`) [pr3390385, pr3390385c, pr3462145].
- unsized constants inside concatenations (`{a - 1}`) are accepted; the standard forbids
  them and the reference reports an error.
- `unique` case/if: overlapping items are not reported (only "no item matches", as the
  reference); `foreach` over hierarchical names or class/queue members; `inside` with
  unpacked arrays as set items.
- `string` (first version, 2026-11-14): no reclaiming of strings (every distinct text made at
  run time stays in the table: a testbench building millions of different strings grows);
  string parameters; strings as `case` expressions; implicit string -> vector assignment is
  accepted (the reference rejects it); characters above 127 print as UTF-8; `%v` `%u` `%z`
  in `$sformatf` [sformatf].

Verilog / scheduling:
- nets with several drivers, `wand`/`tri1`, strength drivers and `assign z = m[i]` update in
  the combinational settle, the reference updates them immediately (see Differences).
- `~`/`buf` of a never-driven input gives x, the reference z (ICARUS_NOTES 8) [sv_unit1c].
- strength values on nets (`%v`, strength reduction through switches, `trireg` charge),
  `$countdrivers`, `$fread`, `%u`/`%z` in `$scanf`, event-controlled NBAs updating at the
  trigger (ICARUS_NOTES 3), specify paths on inout ports and conditional paths, timing
  checks, SDF, force on array elements, generate blocks referenced before their definition.
- compiler warnings the reference prints: port coercion to inout, constant out-of-range
  selects, `@*` without sensitivities.
- `$dist_*` and `$q_*` system functions.
- a second `$dumpvars` does not print "ignoring signals in previously scanned scope";
  `$dumpvars` scope/level arguments; `$dumplimit`, `$dumpports`; FST; VCD of arrays.
- multi-dimensional arrays as ports and in whole-array assignments, array ports connected
  to arrays of another shape (index range, direction), assignment patterns for unpacked
  arrays, a timing control in a whole-array assignment; VPI (A4); configurations and libraries (`config`, `-y`/`-v`,
  `` `uselib ``).
- recursion copies frames (fine for testbench code, slow for deep recursion).
- inout port expressions; port expressions are connected through a continuous assignment,
  not collapsed as the reference does (an extra delta step between the port and the signal).
- interactive/MCP `force` of a net made an alias of its source (`assign z = a`) forces the
  source.
