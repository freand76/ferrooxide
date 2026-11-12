# ferrooxide

A 4-state, event-driven Verilog simulator in Rust. One command, no separate compile step:

```sh
cargo build --release
target/release/ferrooxide tb.v dut.v            # -s top, -D NAME=val, -I dir, --stats, --max-time T
tests/difftest.sh                            # compare against Icarus Verilog on tests/verilog/*.v
```

See [PLAN.md](PLAN.md) for the architecture, milestones and performance plan.

Crates: `ferrooxide-front` (preprocessor, lexer, parser), `ferrooxide-compile` (elaboration, codegen),
`ferrooxide-rt` (4-state ops, scheduler, VM, VCD), `ferrooxide` (CLI).
