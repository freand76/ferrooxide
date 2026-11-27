# ferrooxide

4-state, event-driven Verilog simulator in Rust with a Cranelift JIT. One command parses,
elaborates, compiles and simulates: `target/release/ferrooxide [opts] files... [+plusargs]`.
Goal: the behaviour the standard (IEEE 1364-2005 / 1800-2017) gives, measured with chipsalliance
sv-tests; ≥10x the speed of Icarus Verilog (the speed baseline only, not a reference for
behaviour). Plan and status: `PLAN.md`.

## Layout
- `crates/ferrooxide-front`: preprocessor, lexer, parser, AST (UDP tables are lexed as raw text).
  `syntax/`: the IEEE 1800 parser (keywords, tokens, syntax tree; it knows nothing but type
  names); `lower/`: syntax tree -> AST (type tables, enums, packages, expansions), reporting
  what it cannot do at its line.
- `crates/ferrooxide-compile`: `lib.rs` (compiler state in sub-structs, scopes, name lookup,
  constant evaluation), `types.rs` (`Type` per signal, `type_of(expr)`, packed layouts,
  struct member chains, assignment patterns), `elab/` (hierarchy, params, declarations, the
  item walk, nets with several drivers and links, UDPs, specify), codegen to `Instr` bytecode
  (`codegen.rs`, `expr.rs`, `stmt.rs`, `strings.rs`, `heap.rs` queues/dynamic/associative
  arrays, `class.rs`, `vif.rs` virtual interfaces), `konst.rs`, `lint.rs`.
- `crates/ferrooxide-rt`: `vm.rs` (scheduler + interpreter), `fast.rs` (lowering to `Op`),
  `jit.rs` (Cranelift, one function per process *shape*), `fuse.rs`, `ops.rs`, `fmt.rs`, `vcd.rs`,
  `history.rs` (value change log for the debug front ends).
- `crates/ferrooxide`: CLI (`main.rs`, `session.rs`), `-i` (`console.rs`, `repl.rs`), `mcp.rs`,
  `cdsp.rs`. `tests/suite/<area>/`: own test suite. `bench/`: benchmarks.

## Commands
```sh
cargo build --release
tests/suite.sh [-v] [files|dirs]     # own suite (tests/suite/<area>), all three modes, no iverilog
cargo test --release                 # own suite + lint test + parser snapshots
FX_BLESS=1 cargo test --release -p ferrooxide-front --test parse   # rewrite parser snapshots
tests/parsecheck.py [--baseline]     # parser (--parse-only) over sv-tests, the regression
                                     # suite, tests/ and real designs; changes vs the baseline
tests/svtests.py [--baseline] [--tags]  # sv-tests as its ferrooxide runner runs them (all
                                     # modes, :assert: checks); changes vs the baseline
cargo build --profile speed          # with LTO, for speed measurements only (target/speed/)
perf stat -e cycles target/speed/ferrooxide bench/cpuish.v   # perf works on this machine
```
Debug: `--dump-ast` (parsed tree, Verilog-like), `--dump-code`, `--stats`, `FX_TRACE=<time>`
(activations), `FX_GENSTATS=1` (generic-path instructions, notifies), `FX_MAX_MEM_MB` (design
memory limit, default 2048), `FX_MEM_LIMIT_MB` (address-space cap of the process, default
4096, 0: none).

## Rules
- Every new feature gets a small test that passes in all three modes (default, `--no-jit`,
  `--no-fuse`): in `tests/suite/<area>/` (self-checking, or `// expected` with a checked-in
  `.expected`; behaviour from the standard, never from another simulator; `// args: +x=1`
  passes plusargs). The goal is passing sv-tests: run `tests/svtests.py` after each step,
  no test worse than the baseline.
  New syntax also gets a parser snapshot in `crates/ferrooxide-front/tests/parse/` (a file,
  or a directory for a multi-file unit, with its `.ast`); review the `.ast` diff when blessing.
- Front end: the parser (`syntax/`) builds what the source says and resolves no names except
  type names (needed to parse `T x;`, `T'(e)`, `$bits(T)`); everything else (types, members,
  packages, expansions) belongs in the lowering (`lower/`) or the compiler. New syntax gets
  its own tree node (`syntax/tree.rs`), never a `$__` pseudo call or a reuse of an unrelated
  node. The lowering reports what it cannot handle as "X is not supported" at its line; it
  never drops or approximates a construct silently. `--parse-only` / `tests/parsecheck.py`
  check the parser on its own; `--elab-only` stops after elaboration and compilation.
- Check benchmark cycles after scheduler/JIT changes (with the `speed` profile); don't regress them.
- Memory: this machine has 14 GB and an out-of-memory crash kills the session (it has happened).
  EVERY run of ferrooxide, iverilog/vvp (speed comparisons) or a test outside the scripts
  goes under a limit: `(ulimit -v 2000000; timeout 60 target/release/ferrooxide ...)`.
  ferrooxide caps itself at `FX_MEM_LIMIT_MB` (default 4096); suite.sh and svtests.py set
  limits. Long or parallel jobs run in the background. Never wait for a build with
  `pgrep -f "cargo build"` loops (they match themselves); just run `cargo build`.
- Commit finished, tested steps locally; push only when asked.
- Commit messages: title of about 50 characters (imperative, no period), blank line, body
  wrapped at 70 columns. The body says clearly what was done and mostly why, not how; no
  reasoning or narrative. Don't name other simulators or private designs.

## Standard deviations
`DEVIATIONS.md` lists places where ferrooxide copied Icarus's departures from the standard
(from when Icarus was the reference); change them to the standard when a test needs it.
ferrooxide defaults to SystemVerilog rules; `-g2005` selects Verilog-2005 where they differ.

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
