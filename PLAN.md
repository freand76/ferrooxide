# ferrooxide — plan

## Goals
- IEEE 1364-2005 Verilog and a usable SystemVerilog (1800-2017) subset: RTL and testbenches.
- Full 4-state semantics and the IEEE stratified event scheduler.
- **≥10x Icarus (vvp) throughput** on typical RTL + testbench workloads. Icarus is the
  baseline for speed only.
- The standard is the reference for behaviour (since 2026-11-25; before, the output matched
  Icarus). Coverage is measured with chipsalliance *sv-tests* (`tests/svtests.py`), behaviour
  is checked by our own suite (`tests/suite`). Where ferrooxide still copies an Icarus
  deviation (`DEVIATIONS.md`) it changes to the standard when a test needs it.
- One command, no intermediate files: `ferrooxide tb.v dut.v` parses, elaborates, compiles
  (milliseconds) and simulates.

## Status (2026-11-26)
- sv-tests: 4900 of 5112 (95.9%); all of chapters 5 and 6. Baseline in
  `target/svtests-baseline.res`.
- Speed: 13x (cpuish) and 19x (mixed) fewer cycles than Icarus; the real-design benchmark
  10-13x.
- Debug front ends: interactive mode, MCP server, `--vcd`/`--trace`, CDSP server with value
  history.

## Architecture

```
 .v/.sv files
     │  ferrooxide-front    preprocessor → lexer → syntax/: IEEE 1800 parser (syntax tree, only
     │                      type names resolved) → lower/: tables, enums, packages, classes,
     ▼                      expansions; "X is not supported" at its line
   AST  (declarations keep their data type; `Expr::Member`, `Expr::Pattern`)
     │  ferrooxide-compile  elaboration: params, hierarchy flattening, port collapsing, a `Type`
     │                      per signal and `type_of(expr)`, packed layouts, IEEE width/sign
     ▼                      rules, codegen to bytecode (subroutines inlined)
  Program  (flat memory image + per-process bytecode + static fan-out tables + classes)
     │  ferrooxide-rt       event scheduler + bytecode VM, lowering to fast ops, fusion,
     ▼                      Cranelift JIT; string table and object heap (generic path)
  simulation
```

Key design decisions:
1. *Flat memory*: every signal, temporary and constant lives in one `Vec<u64>`; a value is
   `n` value words + `n` unknown words (00=0, 10=1, 01=z, 11=x), bits above the width 0.
2. *2-state fast path*: ops on ≤64-bit values check the unknown words once, then do plain
   machine arithmetic.
3. *Static everything*: temporaries per process, static event-control ids, a precomputed
   signal→waiter fan-out table; waking a process is a compare and a push.
4. *Port collapsing*: ports connected to whole signals or constant-index elements alias them.
5. *Processes are coroutines with one `pc`*; process *instances* (a fork branch started again
   while it runs) get their own id, JIT table and swapped-in frame.
6. *Rank-ordered combinational settle*: each dirty combinational process runs once per settle
   in rank order (from digsim-native); loops fall back to events.
7. *Cranelift JIT per process shape*: identical processes of different instances share code.
8. *Handles for dynamic data*: strings, queues, dynamic/associative arrays and class objects
   are one 64-bit word (0 = empty / `null`); their operations are generic-path instructions.

Scheduling order where IEEE leaves it open (kept from the Icarus days): wakes FIFO between
active processes and the settle, waiters of one change newest first, fork branches last to
first, time-0 processes by scope, `always_comb` after all others at time 0 (9.2.2.2.2).

## Done
- Verilog-2005: modules/ports (ANSI, non-ANSI, port expressions), parameters, generate,
  hierarchical names, `defparam`, arrays of instances, all operators with IEEE width/sign
  rules, functions/tasks (inlined; recursion through saved frames), fork/join, events,
  `disable`, force/release, nets with several drivers, strengths, `tran`, UDPs, gate and
  net delays, specify paths, `timescale`, reals, file I/O, plusargs, VCD.
- SystemVerilog data: typedefs, enums, packed and unpacked structs/unions (also array
  members), casts, assignment patterns (positional, keys, `default:`, type keys,
  replication), packages (import/export), `$unit`, interfaces/modports, strings, queues,
  dynamic and associative arrays with their methods, streaming concatenations, `chandle`,
  2-state types, unpacked array parameters.
- Classes: properties, constructors, inheritance, virtual/pure/extern/static methods,
  parameterized classes, forward declarations, static members through type parameters,
  constraints and `randomize()`, `randcase`, mailbox, semaphore, `std::process`
  (self/status/kill/await).
- Procedural: `always_*`, `final`, `unique`/`priority`, `inside`, `foreach` (also over
  members, strings and queues; resolved in the compiler), `ref` arguments (by copy, see
  Unsupported), `wait fork`, programs.
- Assertions: immediate, deferred and concurrent (`assert`/`assume`/`cover property`,
  sequence and property operators, named sequences/properties, default clocking,
  `disable iff`, `$past` & co.), elaboration system tasks.
- Performance: fast ops, JIT per shape, scratch temporaries, element-level sensitivity,
  fusion of combinational chains, edge groups, wide ops in the JIT.
- Debug front ends: interactive mode (`-i`), MCP server (`--mcp`), CDSP debug server
  (`--cdsp`) with value history, `--vcd`/`--trace`, `--lint`.
- Restructuring (R0-R6): a complete IEEE 1800 parser with a syntax tree and a separate
  lowering; the compiler split into elaboration, types and codegen; `Type`/`type_of` in the
  compiler; spans on expressions; one declared type per parameter (R4 slice 5); declarations
  and function returns keep only their type, kind, sign and ranges derived from it.
- For UVM (2026-11-26): class typedef scopes (`C::type_id::create()`), DPI exports, static
  properties of parameterized classes (per specialization, inherited), enum methods through
  type parameters, bases specialized with the class's parameters or declared later,
  queues/associative arrays of queues and arrays of several dynamic dimensions, typedefs in
  subroutines, method calls on static calls' results.
- Process: own test suite in three modes (no other simulator needed), parser snapshots,
  sv-tests runner with baseline, regenerated sv-tests (`make generate-tests`, Python
  requirements in a scratch venv).

## Next (in priority order)
UVM must run eventually (item 5): new work keeps it possible. In particular, class
features must work per specialization and for classes declared later, and the event/wait
machinery must not assume static targets only (A3).

1. **sv-tests features, by tests won**, chapter by chapter outside UVM (18: 3, 16: ~29,
   11: 2, 20: 5). DPI (9) needs a C interface: last.
2. **Known bugs**: 2-state members (`int`) of unpacked structs start as x and keep x bits
   (the struct is one 4-state signal; `Packed` has no 2-state flag, which type keys of
   patterns also lack); enum methods of a call's result (`p.status().name()`) and of a
   struct member (`r.size.name()`).
3. **sv-tests invalid code** (8 accepted that must fail): see the ACCEPT lines of the
   baseline; errors in class methods that are never called.
4. **Real subroutine calls and dynamic waiters** (A3): needed for `ref` as a real
   reference, `process::suspend`/`resume`, recursion from fork branches, waits on run-time
   targets other than event properties (`vif.clk`), VPI callbacks and MCP conditional
   breakpoints. The gate for UVM. (Event properties wait through per-object waiter lists,
   `EvOp::Arm`.)
5. **UVM** (157 sv-tests, run with `-DUVM_NO_DPI`; after A3). Stops at
   `uvm_report_message.svh:567` (`process::get_randstate`: per-process random generators,
   IEEE 1800-2017 18.14, replacing deviation 5 of `DEVIATIONS.md`). Then fix the blockers
   one at a time, each with a test. Reproduce with the command `tests/svtests.py` builds for a `uvm` test (e.g.
   chapter-16/16.10--property-local-var-uvm.sv).
6. **R4, the rest**: the compiler reads the syntax tree's types directly and most of the
   lowering goes. Large; only when feature work slows down.
7. **Deviations copied from Icarus** (`DEVIATIONS.md`): switch to the standard where a test
   needs it.
8. **Debug front ends**: report the Surfer bugs upstream; MCP over HTTP, MCP history queries,
   CDSP arrays, port directions and diagnostics; interactive watch/conditional breakpoints.
9. **Performance** (one instructions check after runtime changes; deeper work when speed
   matters again): `notify` (largest item), a timing wheel for `Sim::future`, wide ops that
   leave the JIT, inline resolution of multi-driver nets, the A3 instance checks.
10. **Outputs and Verilog-2005 rest**: FST, VCD of arrays, `$dumpvars` arguments; VPI (A4),
    configurations and libraries, timing checks, SDF.
11. **Multi-threading** (A5): last; pays off only on large designs.

## Architecture changes ahead
- **A3. Real calls, dynamic waiters.** Process instances are done. Left: subroutines that are
  called instead of inlined (methods through a handle, large functions), and a per-signal list
  of dynamic waiters next to the static fan-out (a flag keeps the static path fast).
- **A4. VPI** (cocotb): a handle tree from the hierarchy model, get/put value through the
  force-mask path, value-change callbacks as dynamic waiters, region hooks in the scheduler.
- **A5. Multi-threading**: needs per-thread scratch (a base pointer in the JIT ABI),
  per-thread NBA queues merged in a fixed order, parallel work only for processes without
  system tasks.
- **Heap**: nothing is freed (strings, objects); reference counts or a collector when a
  testbench needs it.
- **CDSP later**: checkpoints and re-simulation for history, setting values, attributes;
  upstream proposals (4-state values, incremental `query_interval`).
- **Watch**: elaboration and codegen run per instance; measure on a large netlist.

## Testing
- `tests/suite/<area>/*.v` (`tests/suite.sh`, in `cargo test`): every test in all three modes
  (default, `--no-jit`, `--no-fuse`); self-checking (`PASSED`) or `// expected` with a
  checked-in `.expected` (`--bless` writes it: review it). Behaviour from the standard. The
  `verilog` area holds the former Icarus diff tests (outputs taken from Icarus).
- Pitfalls: fork branches start last to first (a trigger for another branch needs `#0`);
  `join_any` leaves the other branches running; a port without a direction inherits the
  previous one (`ref int a, int b`: b is ref); a task argument is context-determined by the
  port width.
- Parser snapshots: `crates/ferrooxide-front/tests/parse/` (`--dump-ast`), bless with
  `FX_BLESS=1`.
- sv-tests: `tests/svtests.py [--baseline] [--tags] [--filter RE]`; no test worse than the
  baseline after a step. `tests/parsecheck.py` runs the parser alone over sv-tests, the Icarus
  regression suite (as a corpus) and real designs.
- `bench/`: benchmarks; `perf stat -e instructions,cycles` with the `speed` profile.

## Performance notes
CPU cycles (`perf stat`, `speed` profile, 2026-11-20, median of three):

| benchmark | Icarus | ferrooxide JIT | `--no-jit` | speed-up |
|-----------|-------:|---------------:|-----------:|---------:|
| `bench/cpuish.v` (32 lanes LFSR+ALU, 200k cycles) | 33.6 G | 2.60 G | 4.33 G | 12.9x |
| `bench/mixed.v` (comb chain, 128-bit datapath, memory, FSM, 300k cycles) | 13.2 G | 0.68 G | 1.13 G | 19.4x |

With a full VCD dump the lead is 10-14x. Profile (2026-11-14): `Sim::notify` 25-36 %, JIT code
25-39 %, `advance_steps` 12-19 %, generic instructions 7-10 %, the future queue up to 7.5 %.
Cycles move by up to ±4 % between builds with identical instruction counts (code layout):
compare instructions too.

## Debug front ends (reference)
- **CDSP** (https://cxxrtl.org/protocol.html, v0.14.0): NUL-terminated JSON over TCP/Unix
  sockets; `cdsp.rs` runs the simulation in chunks of 10-40 ms and checks commands between
  time steps; history in `rt/history.rs` (one change log, budget `FX_HISTORY_MB`). Values are
  two-state in the protocol (x→1, z→0).
- **Surfer as client**: needs all scopes in one `list_scopes`, answers strictly in order,
  re-queries `[0, latest]` after every status change. Surfer 0.7.0 shows no live waves
  because of two bugs (stale `cached_time_range`, `signal_id` not implemented for CXXRTL),
  fixed locally in `/projects/surfer`, to report upstream.
- **MCP** (`--mcp`, JSON-RPC 2.0 over stdio): load, lint, run, step, values, force/release,
  watch, breakpoints, trace; results small and bounded, errors with locations.

## Differences from Icarus (the former reference)
- A ranked combinational process runs before the other waiters of a change.
- Some net updates are immediate in Icarus; here only `assign z = a`-style aliases.
- An event-controlled NBA (`x <= @(e) v`) updates after the event, as the standard says.
- Unpacked arrays of opposite directions pair from the left (7.6).
- The VCD is not byte-identical (variable order, port codes, net kinds).

## Unsupported (known)
Kept up to date as features stop short.

SystemVerilog data and testbench:
- Function results of several unpacked dimensions (as local variables of subroutines);
  an unpacked array result in a continuous assignment or a method call.
- `ref` arguments are copied in and out; calls where that differs from a reference are
  rejected (a task with ref arguments that can wait, a ref argument whose variable the
  subroutine also names) [sv-tests chapter-7/queues/persistence]. Needs A3.
- `process`: `suspend`/`resume`, `srandom`/`get_randstate`/`set_randstate`;
  `process::self().kill();` as a statement.
- unpacked array members of unpacked structs are laid out as packed arrays of their
  elements (`$size` & co. and `%p` see a packed array); real, string and unpacked-struct
  elements; member defaults; type keys in patterns of unpacked arrays.
- fixed-size arrays of structs as class properties; writes to a member of a
  struct property at a run-time index (`obj.s.a[i] = v`) or to a tagged union property's
  member; fixed-size arrays of queues, arrays of queues as
  class properties or ports, `%p` of an element array (`$display("%p", a[k])`), queue ports, `%p` of unpacked
  structs, associative arrays and class handles, array methods of associative,
  multi-dimensional and struct arrays, foreach over class handle keys (the key
  variable is a vector), queue element writes with `<=` (done at once).
- classes: `->>` of an event property, events of a subroutine or module assigned to an
  event property, queues and dynamic arrays of events; assignments through a call's result
  (`C::get().v = 1`), an untyped `mailbox`, members of
  `P::f().p`, enum
  methods on an enum property from outside the class, whole array properties, class handle
  ports, NBAs to properties, `randc`, `rand_mode`, `constraint_mode`,
  `disable soft`, rand dynamic arrays and real/string/class rand
  properties, `randomize(null)`.
- randsequence: productions that return values or use themselves (it is expanded), `rand
  join` of productions with arguments, alternatives or `return`; the `rand join` weight
  does not bias the interleaving (each is equally likely).
- programs run in the Active region; no implicit `$finish` when all program blocks end.
- `static` variables at the top level of an automatic subroutine; multi-dimensional arrays as
  subroutine arguments; array arguments of recursive calls.
- slices with run-time bounds, `$is_signed`.
- nested modules with ports instantiated from outside; multi-dimensional interface port
  arrays; modport directions are not enforced.
- assignments to properties of a package's class handle (`P::c.x = 1`); `import` in
  subroutines and in blocks of a package that exports names.
- `.triggered` only on events declared in a module; `iff` only on a whole event control.
- nets of a nettype with a resolution function, nettypes
  from packages; interconnect nets connect only 4-state ports of their own width.
- `type(e)` of expressions other than variables and operators on them.
- assertions: inside procedures, local variables of sequences and properties, match items,
  `first_match`, `nexttime`/`always`/`eventually`/`until`/`accept_on` & co., `#-#`/`#=#`,
  `case` properties, strong sequences, several clocks, `.triggered`/`.matched` of sequences,
  `expect`, clocking blocks with clocking signals; a deferred assertion run several times
  in one activation reports only the last outcome. Global clocking: only the module's own
  block (not one of an enclosing module), the `_gclk` functions only in assertions clocked
  by it; an assertion with future values (`$future_gclk`, `$rising_gclk`, ...) decides an
  attempt at the next tick (its action blocks run then, and `disable iff` is that of the
  next tick).
- DPI; a compound assignment in an expression to a target with
  side effects (`(x[i++] += 1)`).
- streaming concatenations: fixed-size unpacked arrays, associative arrays, and queues or
  dynamic arrays of reals, strings or class handles as items; `with` after the first
  queue or dynamic array of a target.
- tagged unions: `unique case matches` and `unique`/`priority if` with a pattern, a
  pattern in a case item's guard, patterns of arrays, a cast of a `tagged` expression
  (`T'(tagged m v)`); pattern matching only of variables; a real or string member of an
  unpacked tagged union is read without the tag check.
- `->>` with a delay or event control, `->>` of an event that is no module's;
  `$typename` of enums, structs, unions, classes and dynamic types.
- strings are never reclaimed; non-UTF-8 bytes of a `string` print as U+FFFD; `%v` in
  `$sformatf`.

Verilog / scheduling:
- nets with several drivers, `wand`/`tri1`, strength drivers and `assign z = m[i]` update in
  the combinational settle.
- strength values on nets (`%v`, `trireg`), `$countdrivers`, specify
  paths on inout ports and conditional paths, timing checks, SDF, force on array elements.
- `$sscanf` details (x/z digits, `%t`, reals, partial failures); `$value$plusargs` warnings.
- hierarchical names of variables in a named block of a later process.
- `` `line `` only sets `` `__LINE__ `` / `` `__FILE__ ``.
- `$dumpvars` scope/level arguments, `$dumplimit`, `$dumpports`, FST, VCD of arrays.
- multi-dimensional arrays as ports and in whole-array assignments, array ports of another
  shape, a timing control in a whole-array assignment; VPI; configurations and libraries.
- recursion copies frames (slow for deep recursion).
- inout port expressions; port expressions add a delta step.
- interactive/MCP `force` of an aliased net (`assign z = a`) forces the source.
