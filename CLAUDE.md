# ferrooxide

4-state, event-driven Verilog simulator in Rust with a Cranelift JIT. One command parses,
elaborates, compiles and simulates: `target/release/ferrooxide [opts] files... [+plusargs]`.
Goal: output identical to Icarus Verilog, ≥10x its speed. Plan and status: `PLAN.md`.

## Layout
- `crates/ferrooxide-front`: preprocessor, lexer, parser, AST (UDP tables are lexed as raw text).
  `syntax/`: the IEEE 1800 parser (keywords, tokens, syntax tree; it knows nothing but type
  names); `lower/`: syntax tree -> AST (type tables, struct layouts, patterns, enums,
  packages, expansions), reporting what it cannot do at its line.
- `crates/ferrooxide-compile`: elaboration (params, hierarchy, generate, port aliasing, nets with
  several drivers, force), codegen to `Instr` bytecode (`lib.rs`, `expr.rs`, `stmt.rs`, `lint.rs`).
- `crates/ferrooxide-rt`: `vm.rs` (scheduler + interpreter), `fast.rs` (lowering to `Op`),
  `jit.rs` (Cranelift, one function per process *shape*), `fuse.rs`, `ops.rs`, `fmt.rs`, `vcd.rs`.
- `crates/ferrooxide`: CLI. `tests/verilog/*.v`: Icarus diff tests. `bench/`: benchmarks.

## Commands
```sh
cargo build --release
tests/difftest.sh [files]            # vs iverilog/vvp; FX_FLAGS=--no-jit / --no-fuse for other modes
tests/suite.sh [-v] [files|dirs]     # own suite (tests/suite/<area>), all three modes, no iverilog
cargo test --release                 # difftest + own suite + lint test + parser snapshots
FX_BLESS=1 cargo test --release -p ferrooxide-front --test parse   # rewrite parser snapshots
tests/ivtest.py <copy of ivtest dir> # Icarus regression suite (use a copy: tests write files)
tests/regress.sh [--baseline]        # both regression lists, per-test changes vs the baseline
tests/parsecheck.py [--baseline]     # parser (--parse-only) over sv-tests, the regression
                                     # suite, tests/ and real designs; changes vs the baseline
perf stat -e cycles target/release/ferrooxide bench/cpuish.v   # perf works on this machine
```
Debug: `--dump-ast` (parsed tree, Verilog-like), `--dump-code`, `--stats`, `FX_TRACE=<time>`
(activations), `FX_GENSTATS=1` (generic-path instructions, notifies), `FX_MAX_MEM_MB` (design
memory limit, default 2048), `FX_MEM_LIMIT_MB` (address-space cap of the process, default
4096, 0: none).

## Rules
- Every new feature gets a small test that passes in all three modes (default, `--no-jit`,
  `--no-fuse`): in `tests/suite/<area>/` (self-checking, or `// expected` with a checked-in
  `.expected`; behaviour from the standard) and/or in `tests/verilog/` when matching Icarus's
  exact output matters (diffed against it; `// args: +x=1` passes plusargs to both).
  New syntax also gets a parser snapshot in `crates/ferrooxide-front/tests/parse/` (a file,
  or a directory for a multi-file unit, with its `.ast`); review the `.ast` diff when blessing.
- Front end: the parser (`syntax/`) builds what the source says and resolves no names except
  type names (needed to parse `T x;`, `T'(e)`, `$bits(T)`); everything else (types, members,
  packages, expansions) belongs in the lowering (`lower/`) or the compiler. New syntax gets
  its own tree node (`syntax/tree.rs`), never a `$__` pseudo call or a reuse of an unrelated
  node. The lowering reports what it cannot handle as "X is not supported" at its line; it
  never drops or approximates a construct silently. `--parse-only` / `tests/parsecheck.py`
  check the parser on its own.
- Check benchmark cycles after scheduler/JIT changes; don't regress them.
- Memory: this machine has 14 GB and an out-of-memory crash kills the session (it has happened).
  EVERY run of ferrooxide, iverilog/vvp or a test outside the scripts goes under a limit:
  `(ulimit -v 2000000; timeout 60 target/release/ferrooxide ...)`. ferrooxide caps itself at
  `FX_MEM_LIMIT_MB` (default 4096); difftest.sh and ivtest.py set limits (ivtest: 2 GB per test,
  4 at a time). Long or parallel jobs run in the background. Never wait for a build with
  `pgrep -f "cargo build"` loops (they match themselves); just run `cargo build`.
- Commit finished, tested steps locally; push only when asked.
- Commit messages: title of about 50 characters (imperative, no period), blank line, body
  wrapped at 70 columns. The body says clearly what was done and mostly why, not how; no
  reasoning or narrative. Don't name the reference simulator, its test suite or private
  designs; say "the reference" / "the regression suite".

## Icarus quirks
`ICARUS_NOTES.md` lists places where Icarus seems to deviate from the standard (we match it);
add to it when you find one. ferrooxide defaults to SystemVerilog rules; `-g2005` selects
Verilog-2005 where they differ (ivtest.py passes it unless a test asks for SV).

## Invariants worth knowing
- Value = `nw` value words + `nw` unknown words (00=0, 10=1, 01=z, 11=x); bits above the width
  are always 0. Slots below `Program::scratch_words` are shared temporaries, dead at every
  suspension point (`ptmp` for values that live across waits).
- Scheduling mimics Icarus where IEEE leaves order open: FIFO between active wakes and the
  combinational settle (sequence numbers), newest waiter first (`wseq`), fork branches last to
  first, combinational processes run before others at time 0.
- Fusion (`fuse.rs`) only for pure combinational processes with blocking writes whose outputs
  nothing else writes; otherwise it can loop.
- Stores to forced signals must take the generic path (force mask); `fast::lower` checks
  `SignalInfo::forced`.
- JIT code reads `ProcRt` (pc@0, wait@4, wseq@8) and `Hot` field offsets directly; keep them in
  sync with `jit.rs`.
