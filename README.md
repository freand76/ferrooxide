# ferrooxide

A 4-state, event-driven Verilog simulator in Rust with a Cranelift JIT. One command, no separate
compile step (about 10x faster than Icarus Verilog on RTL benchmarks, see PLAN.md):

```sh
cargo build --release
target/release/ferrooxide tb.v dut.v            # -s top, -D NAME=val, -I dir, --stats, --max-time T, --no-jit
tests/difftest.sh                            # compare against Icarus Verilog on tests/verilog/*.v
FX_FLAGS=--no-jit tests/difftest.sh          # same, with the interpreter
```

See [PLAN.md](PLAN.md) for the architecture, milestones and performance plan.

Crates: `ferrooxide-front` (preprocessor, lexer, parser), `ferrooxide-compile` (elaboration, codegen),
`ferrooxide-rt` (4-state ops, scheduler, VM, VCD), `ferrooxide` (CLI).
