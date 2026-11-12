# ferrooxide

A fast, 4-state, event-driven Verilog simulator written in Rust, with a Cranelift JIT.

One command, no separate compile step: `ferrooxide tb.v dut.v` parses, elaborates, compiles the
design to native code in milliseconds and simulates it. Output is compatible with Icarus Verilog,
which is used as the reference in the test suite.

```sh
cargo build --release
target/release/ferrooxide tb.v dut.v
```

## Performance

CPU cycles (`perf stat`) on the same machine, Icarus Verilog 14 (`iverilog` + `vvp`) vs ferrooxide:

| benchmark | Icarus | ferrooxide | speed-up |
|-----------|-------:|-----------:|---------:|
| [`bench/cpuish.v`](bench/cpuish.v): 32 lanes of LFSR + ALU + accumulator, 200k clock cycles | 33.6 G | 2.96 G | **11x** |
| [`bench/mixed.v`](bench/mixed.v): combinational chain, 128-bit datapath, memory, FSM, 300k cycles | 13.5 G | 0.66 G | **20x** |

Both produce output identical to Icarus.

How: 4-state values as (value, unknown) word pairs with a 2-state fast path; processes lowered to
specialised ops and JIT-compiled per *shape* (identical processes of different instances share
native code); temporaries in registers; combinational logic evaluated once per settle in rank
order (and fused into fewer processes); clocked processes on the same edge run as one group;
element-level sensitivity for arrays; ports collapsed onto the signals they connect to. See [PLAN.md](PLAN.md) for details and measurements.

## Language support

Verilog-2005 plus common SystemVerilog:

- modules, ANSI and non-ANSI ports, `parameter`/`localparam` with overrides, `generate`
  for/if/case, gate primitives, hierarchical names
- `wire`/`reg`/`integer`/`time`, SV `logic`/`bit`/`int`/`byte`/`shortint`/`longint`,
  vectors of any width, memories (one unpacked dimension)
- `assign`, `initial`, `always`, `always_comb`, `always_ff`, `always_latch`, `final`
- blocking/non-blocking assignments (including `<= #d`), `if`, `case`/`casez`/`casex`,
  `for`/`while`/`do`/`repeat`/`forever`, `#` delays, `@` events, `wait`, named blocks + `disable`,
  `break`/`continue`, `++`/`+=`
- functions and tasks (inlined), all operators with IEEE width/sign rules and x/z semantics
- `` `timescale `` (per module, fractional delays), `$time`/`$stime`/`$realtime`, `%t`
- `real`/`realtime` variables, arrays, parameters, task arguments and function results
- nets with several drivers (tri-state buses through `inout` ports), hierarchical names
  (reads, writes, parameters, task calls), `force`/`release` (as assignment)
- `$display`/`$write`/`$monitor`/`$strobe`/`$finish`/`$random`/`$signed`/`$unsigned`/`$clog2`/
  `$readmemh`/`$readmemb`/`$fopen` (modes)/`$fdisplay`/`$fwrite`/`$fclose`/`$sformat`,
  `$test$plusargs`/`$value$plusargs` (`+name=value` on the command line), `$error`/`$fatal`,
  `%f`/`%e`/`%g`
- VCD output: `$dumpfile`, `$dumpvars`, `$dumpon`, `$dumpoff`
- preprocessor: `` `define `` (with arguments), `` `ifdef ``/`` `ifndef ``/`` `elsif ``/`` `else ``, `` `include ``

Not yet: `fork`/`join` (branches run sequentially, with a warning), drive strengths, `force`
holding its value, `$fgetc`/`$fscanf`, named events, arrays of instances, multi-dimensional
arrays. See the milestones in [PLAN.md](PLAN.md).

## Usage

```
ferrooxide [options] <files...>
  -s <top>          top-level module (may be repeated; default: all uninstantiated modules)
  -D <name>[=val]   define a preprocessor macro
  -I <dir>          add an include directory
  --max-time <t>    stop simulation after time t
  --stats           print timing and scheduler statistics to stderr
  --dump-code       print the compiled bytecode to stderr
  --no-jit          interpret the bytecode instead of compiling it to native code
  --no-fuse         do not fuse combinational processes
  --lint            check the design and exit (status 1 on findings), see below
  +<arg>            plusarg for $test$plusargs / $value$plusargs
```

### Lint

`ferrooxide --lint <files>` elaborates the design without simulating and reports **incomplete
sensitivity lists**: a level-sensitive `always @(a or b ...)` block that reads a signal missing
from its list. Simulation only re-evaluates the block when a listed signal changes, but synthesis
builds combinational logic that follows every input, so simulation and hardware disagree.

```
tb.v:19: lint: incomplete sensitivity list: 'b' is read but not in the list (synthesis treats the block as combinational logic of all its inputs; use @*)
ferrooxide lint: 1 finding
```

Signals read inside called functions, memories and hierarchical references count; variables the
block assigns itself (temporaries), loop variables, edge-triggered blocks and `@*`/`always_comb`
are not reported.

Debugging: `FX_TRACE=<time>` prints every process activation from that time on,
`FX_GENSTATS=1` counts instructions that take the generic path.

## Testing

`tests/difftest.sh` runs every testbench in `tests/verilog/` through Icarus and ferrooxide and
diffs the output (`cargo test` runs it too when `iverilog` is installed). `FX_FLAGS` selects
other execution modes:

```sh
tests/difftest.sh
FX_FLAGS=--no-jit tests/difftest.sh
FX_FLAGS=--no-fuse tests/difftest.sh
```

## Structure

| crate | contents |
|-------|----------|
| `ferrooxide-front` | preprocessor, lexer, parser, AST |
| `ferrooxide-compile` | elaboration (parameters, hierarchy, generate, port collapsing), code generation |
| `ferrooxide-rt` | 4-state ops, scheduler, interpreter, Cranelift JIT, process fusion, VCD |
| `ferrooxide` | command-line tool |

## License

BSD 2-Clause, see [LICENSE](LICENSE).
