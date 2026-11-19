# sv-tests gaps

Results of ferrooxide on [chipsalliance/sv-tests](https://github.com/chipsalliance/sv-tests)
with the ferrooxide runner (`tools/runners/ferrooxide.py`), run on 2026-11-19 at a3fa6b3.

A snapshot: the priorities are in PLAN.md ("Next steps"); the bugs below are fixed (up to
a5d09e5). To be replaced by a script that reruns the tests and regenerates these tables.

The runner maps the sv-tests modes to ferrooxide options:

| sv-tests mode | ferrooxide |
|---|---|
| preprocessing, parsing | `--parse-only` |
| elaboration, simulation_without_run | `--elab-only` |
| simulation | a full run (once per `:simvariants:` entry) |

A test passes when the exit status is 0, or non-zero for a test marked
`:should_fail_because:`. Tests without a `:type:` count as elaboration tests.

**3798 of 5110 tests pass (74.3%).** Of the 1312 that do not:

- 845 elaboration and simulation tests that should pass are rejected (902 including
  parsing and tests with no mode).
- 412 tests that must fail are accepted.
- 52 parsing tests that should pass are rejected.

## Bugs found on the way

These are not missing features; each is a fault in what already exists.

1. **`` `include `` followed by a tab reads the include directory.** `` `include<TAB>"sys.h" ``
   is read as an include of `""`, so the include directory itself is opened: "cannot read
   file: Is a directory". All 34 failing utd-sv parsing tests (e.g.
   `third_party/tests/utd-sv/pcx_databuf_pa.v:29`).
2. **Line numbers after an `` `include `` keep counting the included file's lines.**
   `bsg_wormhole_router_pkg.sv` has 145 lines; its error at line 4 is reported at line 225,
   because the `bsg_defines.sv` it includes on line 1 has 221 lines (194 + 4 = 198 for
   black-parrot's copy). Every error in a file that includes another one points at the
   wrong line.
3. **Directives produced by a macro expansion are not processed.** basejump's
   `` `BSG_DEFIF_NOT_A_OR_B(...) `` expands to `` `ifndef ``/`` `define ``/`` `endif `` lines;
   ferrooxide reports "unterminated `` `ifdef ``" at line 1 of
   `bsg_test/bsg_nonsynth_clock_gen.sv`.
4. **Source that is not UTF-8 cannot be read.** scr1 has Latin-1 comments: "stream did not
   contain valid UTF-8". Non-UTF-8 bytes in comments and strings should be passed through.
5. **Variable initializers must be constant.** A module-level `int c = a.compare(b);`
   (string methods on variables) fails with "not a constant expression" (12 tests, 6.16.x
   string methods). A variable declaration's initializer does not have to be a constant
   expression.
6. **One abort and one timeout** in the regression suite: `regress-vlg_br_gh330_iv.sv` exits
   with SIGABRT (status -6), `regress-vlg_pr587_iv.sv` runs past the 30 s limit.

## Priorities

Ordered by tests won per unit of work, with real designs (the cores) weighted above
single-feature tests. Counts are tests that would stop failing on the first error; some
will then stop at a later one.

| # | Gap | Tests | Why here |
|---:|---|---:|---|
| 1 | Preprocessor bugs 1-3 above (tab after `` `include ``, line numbers after an include, directives from macros) | 35 + every error location | Small fixes; wrong line numbers make every other report harder to act on |
| 2 | Parameters with several packed dimensions (`localparam bit [1:0][4:0][4:0] X = ...`), also as a function return type (rggen) and after an enum | 367 | One construct in one package blocks all 359 basejump tests and the black-parrot ones; packed multi-dimensional vectors already exist for variables |
| 3 | Reject SystemVerilog keywords used as identifiers (in SV mode; keep `-g2005` permissive) | 184 | `--lint` already detects it, so this is mostly turning a finding into an error |
| 4 | Bugs 4-6 above (non-UTF-8 source, non-constant variable initializers, the abort and the timeout) | 15 | Small, and scr1 is a real core |
| 5 | Named arguments in calls (`f(.clear(x), .set(y))`) | 2 | Blocks two real cores (TNoC, RSD); a contained change in call lowering |
| 6 | Checks that reject invalid code: conflicting non-ANSI port declarations, enum base/compatibility rules, `automatic` misuse, constant-expression checks, lvalue selects, real as index or in a concatenation, multiple continuous drivers of a variable, procedural assignment to a net, `local`/`protected` access, macro argument count and `` `line `` arguments | 228 | The other half of "elaboration mode for sv-tests" in PLAN.md. Many small checks; take the largest families first (non-ANSI ports 37, enums 21, automatic 13, constants 11) |
| 7 | Array methods with `with`, locator and ordering methods (`find*`, `sort`, `rsort`, `shuffle`, `reverse`, `unique*`), reduction methods (`sum`, `product`, `and`, `or`, `xor`) | 25 | Already listed as "not yet" in the README; common in testbenches |
| 8 | Concurrent assertions, `sequence`, `property`, `expect`, and clocking blocks | 58 | Used by real cores (ariane) and many §14/§16/§20 tests; large but well-bounded area |
| 9 | Parameterized classes (and `typedef C#(...) T;`, `mailbox #(T)`) | ~171 | Unblocks the first error in all UVM tests (`uvm_pkg.sv`: `typedef uvm_config_db#(...)`), but UVM needs much more after it (constraints, randomize, interface classes); value is mostly long-term |
| 10 | Constraints, `randomize`, `randsequence`, `randcase`, `$dist_*` | 51 | All of §18; also needed for UVM |
| 11 | Remaining SV features, in order of tests: interface classes (21), streaming concatenation (14), extern methods (12), tagged unions (10), assignment in an expression (8), `process` class (4), `$cast`/`$typename` (4), pure methods, replication in assignment patterns, then the single-test items in the table below | ~90 | Each is small in tests; pick by what real designs need |

## Error list

Every error message that fails a test, by the first error the test hit. Names and numbers
in the messages are replaced by `…` and `N`. "§N" is a chapter of IEEE 1800.

### Elaboration and simulation tests that should pass

| Tests | Error | Where (tag group: tests) | Example test |
|---:|---|---|---|
| 365 | a parameter with several packed dimensions is not supported | basejump: 359, black-parrot: 6 | `generated/black-parrot/bp_unicore.sv` |
| 154 | expected a type after typedef, found '…' | uvm: 63, uvm-random: 63, uvm-examples: 28 | `testbenches/uvm_sequence.sv` |
| 30 | a constraint is not supported | §18: 30 | `chapter-18/18.5.2--pure-constraint_0.sv` |
| 29 | a concurrent assertion is not supported | §16: 12, yosys: 6, §20: 6, ariane: 5 | `chapter-16/16.12--property-disable-iff.sv` |
| 21 | an interface class is not supported | §8: 16, §6: 5 | `generic/iface/iface_class_test_10.sv` |
| 20 | a clocking block is not supported | §20: 10, yosys: 6, §14: 4 | `chapter-14/14.3--clocking-block-signals.sv` |
| 15 | a parameterized class is not supported | §6: 11, §8: 4 | `generic/class/class_test_19.sv` |
| 15 | unsupported system function '…' | §20: 10, ivtest: 2, §21: 1, §6: 1 | `chapter-20/20.15--dist_t.sv` |
| 14 | a streaming concatenation is not supported | §11: 14 | `chapter-11/11.4.14.4--dynamic_array_stream_with.sv` |
| 12 | a call with `…` is not supported | §7: 7, §18: 5 | `chapter-7/arrays/associative/locator-methods/find-last.sv` |
| 12 | a extern method is not supported | §8: 12 | `generic/member/class_member_test_2.sv` |
| 12 | not a constant expression | §6: 12 | `chapter-6/6.16.6--string_compare.sv` |
| 12 | randsequence is not supported | §18: 12 | `chapter-18/18.17.1--random-production-weights_0.sv` |
| 10 | a tagged union is not supported | §12: 5, §11: 3, §7: 2 | `chapter-7/unions/tagged/basic.sv` |
| 8 | an assignment in an expression is not supported | §11: 8 | `chapter-11/11.3.6--assign_in_expr.sv` |
| 6 | a sequence declaration is not supported | §16: 5, §9: 1 | `chapter-16/16.9--sequence-noncons-repetition.sv` |
| 6 | an assignment pattern needs a known target type here | §12: 4, §5: 1, ivtest: 1 | `chapter-5/5.10-structure-arrays.sv` |
| 6 | array '…' assigned from something that is not an array of the same shape | §7: 6 | `chapter-7/arrays/unpacked/slice-equality.sv` |
| 6 | cannot resolve hierarchical name '…' | §7: 5, ivtest: 1 | `chapter-7/arrays/unpacked/reduction-methods/product.sv` |
| 6 | no top-level module found | §18: 6 | `chapter-18/18.13.1--urandom_0.sv` |
| 4 | a net of this data type (only N-state types) is not supported | ivtest: 4 | `generated/ivtest/regress-sv_implicit_cast4_iv.sv` |
| 4 | expected a queue or dynamic array value | §7: 4 | `chapter-7/arrays/associative/locator-methods/unique-index.sv` |
| 4 | this data type is not supported | §6: 3, §8: 1 | `generic/member/class_member_test_12.sv` |
| 4 | unknown task '…' | §7: 4 | `chapter-7/arrays/unpacked/ordering-methods/reverse.sv` |
| 4 | unknown type '…' | §9: 4 | `chapter-9/9.7--process_cls_kill.sv` |
| 3 | Scalar port `…`l'…' has a vectored net declaration [N:N]. | ivtest: 3 | `generated/ivtest/regress-vlg_pr1960619_iv.sv` |
| 3 | a compound assignment to a target with side effects is not supported | ivtest: 3 | `generated/ivtest/regress-sv_pr3390385_iv.sv` |
| 3 | a pure method is not supported | §8: 3 | `generic/member/class_member_test_26.sv` |
| 3 | a replication in an assignment pattern is not supported | §5: 3 | `chapter-5/5.11-arrays-replication.sv` |
| 3 | expected '…', found '…' | uvm-examples: 3 | `generated/uvm-examples/biquad_example.sv` |
| 2 | a named argument is not supported | TNoC: 1, rsd: 1 | `generated/tnoc/tnoc.sv` |
| 2 | a nonblocking event trigger is not supported | §15: 2 | `chapter-15/15.5.1--named-event-trigger-blocking.sv` |
| 2 | a parameterized class type is not supported | §15: 2 | `chapter-15/15.4--mailbox-blocking.sv` |
| 2 | a property declaration is not supported | §16: 2 | `chapter-16/16.10--property-local-var.sv` |
| 2 | an `…` event is not supported | ivtest: 1, §9: 1 | `generated/ivtest/regress-sv_edge_iv.sv` |
| 2 | an event as a class property is not supported | §6: 1, §15: 1 | `generic/class/class_test_54.sv` |
| 2 | an unpacked array parameter is not supported | §6: 2 | `generic/class/class_test_49.sv` |
| 2 | randcase is not supported | §18: 2 | `chapter-18/18.16--random-weighted-case-randcase_2.sv` |
| 2 | undeclared identifier '…' | ivtest: 2 | `generated/ivtest/regress-vlg_pr1705027_iv.sv` |
| 2 | unexpected '…' in expression | yosys: 2 | `generated/yosys/simple_specify.sv` |
| 1 | (no message, exit -6 = abort) | ivtest: 1 | `generated/ivtest/regress-vlg_br_gh330_iv.sv` |
| 1 | (no message, exit 71 = timeout) | ivtest: 1 | `generated/ivtest/regress-vlg_pr587_iv.sv` |
| 1 | Scalar port `…`n'…' has a vectored net declaration [N:N]. | ivtest: 1 | `generated/ivtest/regress-vlg_br_gh788_iv.sv` |
| 1 | `…` outside an index of a queue | §20: 1 | `chapter-20/20.6--isunbounded.sv` |
| 1 | a cast to a real type is not supported | ivtest: 1 | `generated/ivtest/regress-sv_cast_real_iv.sv` |
| 1 | a class in a class is not supported | §8: 1 | `generic/member/class_member_test_10.sv` |
| 1 | a const variable is not supported | §6: 1 | `chapter-6/6.20.6--const.sv` |
| 1 | a let declaration is not supported | §11: 1 | `chapter-11/11.12--let_construct.sv` |
| 1 | a rand struct member is not supported | §6: 1 | `generic/typedef/typedef_test_18.sv` |
| 1 | a ref port is not supported | §7: 1 | `chapter-7/queues/persistence.sv` |
| 1 | a return type with several packed dimensions is not supported | RgGen: 1 | `generated/rggen/rggen.sv` |
| 1 | a select of this expression is not supported | §11: 1 | `chapter-11/11.4.12--concat_op-bit_select.sv` |
| 1 | a struct member default is not supported | §7: 1 | `chapter-7/structures/unpacked/default-value.sv` |
| 1 | a user-defined nettype is not supported | §6: 1 | `chapter-6/6.6.7--nettype.sv` |
| 1 | an array of queues or dynamic arrays is not supported | §6: 1 | `generic/typedef/typedef_test_13.sv` |
| 1 | an interconnect net is not supported | §6: 1 | `chapter-6/6.6.8--interconnect.sv` |
| 1 | an unpacked struct member array is not supported | §7: 1 | `chapter-7/arrays/associative/other.sv` |
| 1 | array '…' used without index | ivtest: 1 | `generated/ivtest/regress-sv_br_gh1449_iv.sv` |
| 1 | assignment pattern has no value for member a | §5: 1 | `chapter-5/5.10-structures.sv` |
| 1 | cannot read file: stream did not contain valid UTF-N | scr1: 1 | `generated/scr1/scr1.sv` |
| 1 | expect is not supported | §16: 1 | `chapter-16/16.17--expect.sv` |
| 1 | expected an associative array value | §7: 1 | `chapter-7/arrays/associative/literals.sv` |
| 1 | foreach: '…' is not an array declared here | §6: 1 | `chapter-6/6.6.7--nettype_resolution_fn.sv` |
| 1 | packed dimensions after an enum is not supported | ivtest: 1 | `generated/ivtest/regress-sv_array_packed_iv.sv` |
| 1 | recursive call of '…' from a fork branch is not supported | ivtest: 1 | `generated/ivtest/regress-vlg_recursive_task_iv.sv` |
| 1 | this assignment pattern key is not supported | §5: 1 | `chapter-5/5.11-arrays-key-index.sv` |
| 1 | unconnected UDP terminal | ivtest: 1 | `generated/ivtest/regress-vlg_pr707_iv.sv` |
| 1 | undefined macro ` | ivtest: 1 | `generated/ivtest/regress-vlg_mangle_iv.sv` |
| 1 | undefined macro `SV_COV_CHECK | §20: 1 | `chapter-20/20.14--coverage.sv` |
| 1 | unknown package '…' | §18: 1 | `chapter-18/18.12--randomization-of-scope-variables_0.sv` |
| 1 | unterminated `ifdef | basejump: 1 | `generated/basejump/bsg_test_BaseJumpSTL_bsg_nonsynth_clock_gen.sv` |

### Parsing tests that should pass

| Tests | Error | Where (tag group: tests) | Example test |
|---:|---|---|---|
| 34 | cannot read file: Is a directory (os error N) | utd-sv: 34 | `generated/utd-sv/utd-sv_pcx_databuf_pa.sv` |
| 8 | expected a type after typedef, found '…' | uvm: 8 | `generated/uvm_classes_2/uvm_driver_class_2.sv` |
| 5 | expected '…', found '…' | hdlconv_std2017: 5 | `generated/hdlconv_std2017/hdlconvertor_std2017_p203.sv` |
| 4 | unexpected '…' in expression | hdlconv_std2017: 4 | `generated/hdlconv_std2017/hdlconvertor_std2017_p469.sv` |
| 1 | expected '…' or '…', found '…' | hdlconv_std2017: 1 | `generated/hdlconv_std2017/hdlconvertor_std2017_p490.sv` |

### Invalid code that is accepted

| Tests | Tag | Mode | Why the test must fail (from the test) |
|---:|---|---|---|
| 186 | ivtest | simulation | regression-suite tests that expect a compile error. Largest families: non-ANSI module/task port declarations that conflict (module_nonansi*, task_nonansi*: 37), automatic variables used where static ones are required (automatic_error*: 13), enum base type and compatibility checks (enum_base*, enum_compatibility*, enum_test*: 21), constant-expression checks (check_constant_*: 11), invalid lvalue selects (undef_lval_select*, array_lval_select*: 6), queue/dynamic-array/array assignments of the wrong shape (9), parameter override and specparam misuse (9), real used as an index or in a concatenation (5), bug-report regressions (pr*, br_gh*: 33) |
| 184 | 5.6.2 | elaboration | a SystemVerilog keyword used as an identifier, e.g. `bit property;` (`--lint` already reports it; nothing else rejects it) |
| 6 | 22.5.1 | preprocessing | 22.5.1--define-expansion_12; 22.5.1--define-expansion_21; 22.5.1--define-expansion_23; 22.5.1--define-expansion_6; 22.5.1--define-expansion_7; 22.5.1--define-expansion_8 |
| 6 | yosys | elaboration | this test imported from yosys repository contains intentional syntax error |
| 5 | 22.12 | preprocessing | 22.12--line-illegal-1; 22.12--line-illegal-2; 22.12--line-illegal-3; 22.12--line-illegal-4; 22.12--line-illegal-5 |
| 5 | 8.18 | simulation | 8.18--inherited_local_from_inside; 8.18--inherited_local_from_outside; 8.18--inherited_prot_from_outside; 8.18--local_from_outside; 8.18--prot_from_outside |
| 3 | 6.12 | simulation | 6.12--real_bit_select; 6.12--real_bit_select_idx; 6.12--real_edge |
| 3 | 6.19 | simulation | 6.19--enum_value_inv; 6.19--enum_xx_inv; 6.19--enum_xx_inv_order |
| 3 | 6.5 | simulation | 6.5--variable_mixed_assignments; 6.5--variable_multiple_assignments; 6.5--variable_redeclare |
| 2 | 6.18 | simulation | typedef_test_25__bad; typedef_test_28__bad |
| 1 | 10.3 | simulation | Illegal to procedurally assign to wire, IEEE Table 10-1 |
| 1 | 13.4.4 | simulation | only fork-join_none is permitted inside a function |
| 1 | 22.11 | preprocessing | The pragma specification is identified by the pragma_name, which follows the `pragma directive. |
| 1 | 22.3 | parsing | It shall be illegal for the `resetall directive to be specified within a design element. |
| 1 | 6.19.3 | simulation | enum enforces strict type checking rules |
| 1 | 6.19.4 | simulation | enum numerical expression without casting |
| 1 | 6.20.5 | simulation | specparam assignment to param should be invalid |
| 1 | 7.4.3 | simulation | slicing array with zero part width |
| 1 | 9.3.3 | simulation | illegal return from fork |
