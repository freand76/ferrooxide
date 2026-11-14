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
- nets with several drivers (tri-state buses through `inout` ports, also on parts of nets),
  drive strengths (`assign (pull0, pull1) ...`), `wand`/`wor`/`tri0`/`tri1`/`supply0`/`supply1`,
  `pullup`/`pulldown`, `tran`/`tranif0`/`tranif1`, `wire real`, `` `default_nettype ``
- gate primitives, UDPs, arrays of gate and module instances, `defparam`, hierarchical names
  (reads, writes, parameters, task calls, `inst[2].sig`), `force`/`release`,
  `assign`/`deassign`
- `fork`/`join` (and SystemVerilog `join_any`/`join_none`, `disable fork`), named events
  (`event`, `->`, `@(ev)`), `disable` of named blocks and tasks in any process, `automatic`
  tasks and functions (per-call storage); processes woken together run in Icarus's order
- `$display`/`$write`/`$monitor`/`$strobe` (and their `b`/`h`/`o` and file variants),
  `$finish`, `$random`/`$urandom`/`$urandom_range` (Icarus's sequences), `$signed`/`$unsigned`/
  `$clog2`, `$readmemh`/`$readmemb`/`$writememh`/`$writememb`, `$fopen` (modes)/`$fclose`,
  file input (`$fgetc`/`$fgets`/`$ungetc`/`$feof`/`$fscanf`/`$sscanf`/`$fseek`/`$ftell`/
  `$rewind`), `$sformat`/`$swrite`, `$timeformat`/`$printtimescale`,
  `$test$plusargs`/`$value$plusargs` (`+name=value` on the command line), `$error`/`$fatal`,
  `%f`/`%e`/`%g`
- VCD output: `$dumpfile`, `$dumpvars`, `$dumpon`, `$dumpoff`
- preprocessor: `` `define `` (with arguments), `` `ifdef ``/`` `ifndef ``/`` `elsif ``/`` `else ``, `` `include ``

Not yet: recursive functions, multi-dimensional arrays, `%v` strength display, gate/net
rise/fall delays, `specify` path delays and SDF. See "Road to full Verilog-2005" in
[PLAN.md](PLAN.md). Icarus's regression suite (ivtest) passes at about 83%; places where
Icarus seems to deviate from the standard (ferrooxide matches Icarus) are listed in
[ICARUS_NOTES.md](ICARUS_NOTES.md).

## Usage

```
ferrooxide [options] <files...>
  -s <top>          top-level module (may be repeated; default: all uninstantiated modules)
  -D <name>[=val]   define a preprocessor macro
  -I <dir>          add an include directory
  --max-time <t>    stop simulation after time t
  -q, --quiet       no startup summary on stderr (files, modules, signals, processes and the
                    parse / elaboration / native compile times)
  --stats           print timing and scheduler statistics to stderr
  --dump-code       print the compiled bytecode to stderr
  --no-jit          interpret the bytecode instead of compiling it to native code
  --no-fuse         do not fuse combinational processes
  -g2005            Verilog-2005 rules where SystemVerilog differs (default: SystemVerilog)
  -gspecify         apply specify path delays (ignored by default)
  --vcd <file>      write a VCD without $dumpvars in the testbench (all signals, or --trace)
  --trace <regex>   trace only signals whose hierarchical name matches (repeatable)
  -i, --interactive interactive mode, see below
  --mcp             MCP server for AI agents (JSON-RPC on stdio), see below
  --cdsp [host:]port  live waveforms in a viewer (CXXRTL debug server protocol), see below
  --surfer          start Surfer on the debug server and show the --trace signals, see below
  --lint            check the design and exit (status 1 on findings), see below
  +<arg>            plusarg for $test$plusargs / $value$plusargs
```

### Tracing from the command line

`--vcd FILE` writes a VCD file without `$dumpfile`/`$dumpvars` in the testbench. `--trace REGEX`
(repeatable) limits it to signals whose full hierarchical name matches a regular expression; it
implies `--vcd dump.vcd`:

```
ferrooxide --vcd cpu.vcd --trace '^tb\.cpu\.(pc|ir)$' --trace 'alu\.' tb.v
```

A value is written only when it differs from the last one written for the signal (glitches inside
a time step are not dumped).

### Interactive mode

`ferrooxide -i <files>` compiles the design and then reads commands from stdin (a terminal, or
a script piped in):

```
fx> run 50ns             run to a time (ticks, or with a unit); `run +T` runs T more, `run` to the end
stopped at time 50
fx> print u1.q d         a signal by hierarchical name or unique suffix; radix b, o, d or h
top.q = 8'd4
fx> force q 8'h80        force a value until `release`; `set` assigns once
fx> step                 the current and the next time step
fx> list u1              signals whose name contains the text
fx> release q
fx> watch q d            print q whenever it changes during run/step
fx> break q 8'd5         stop when q becomes 5 (without a value: on any change)
fx> trace w.vcd 'u1\.'   start a VCD of the matching signals now; `trace off` / `trace on`
fx> quit                 run final blocks and exit
```

`help` lists the commands. `$display` output appears as the simulation runs.

### MCP server (for AI agents)

`ferrooxide --mcp [files...]` serves the same operations as the interactive mode as
[Model Context Protocol](https://modelcontextprotocol.io) tools over stdio, so an agent can
debug a design by running and probing it instead of reading logs or dumping waveforms:
`load_design`, `lint`, `run`, `step`, `status`, `get_values`, `list_signals`, `set_value`,
`force`, `release`, `watch`/`unwatch`, `add_breakpoint`/`clear_breakpoints`, `trace` (VCD of
signals matching regular expressions), `trace_control` and `finish`. The design's `$display`
output is returned with each result (at most 200 lines). Register it with an MCP client, e.g.:

```
claude mcp add ferrooxide -- /path/to/ferrooxide --mcp
```

### Live waveforms (debug server)

`ferrooxide --cdsp 6618 <files>` serves the simulation over the
[CXXRTL Debug Server Protocol](https://cxxrtl.org/protocol.html). A waveform viewer attaches
to it, runs and pauses the simulation and shows the history of any signal, with no VCD file and
no `$dumpvars`. With [Surfer](https://surfer-project.org):

```
ferrooxide --cdsp 6618 tb.v &
surfer cxxrtl+tcp://127.0.0.1:6618
```

`ferrooxide --surfer [--trace REGEX...] <files>` does both: it starts Surfer
(`FX_SURFER` selects the executable) and, over Surfer's control protocol (WCP), adds the
signals matching `--trace` (or, without it, the top modules' signals) to the view and puts a
marker at `$finish`. ferrooxide exits when Surfer is closed.

The simulation starts paused after time 0. The value history of every signal is kept in
memory (`FX_HISTORY_MB`, default 512; changes after that are not recorded). The protocol's
value encoding is two-state, so x bits show as 1 and z bits as 0. Reals are 64-bit items
holding the IEEE double (show them with Surfer's 64-bit float format). Arrays are not listed
yet.

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
