# Icarus Verilog: suspected deviations from the standard

ferrooxide copies Icarus's output, including where Icarus seems to depart from IEEE 1364-2005 /
1800. Things found while matching it (Icarus 12, `iverilog`/`vvp`):

1. **No edge from a variable initializer (`-g2005`).** `reg c = 1'b1;` wakes `@(c)` at time 0
   but not `@(posedge c)`, although x→1 is a posedge (1364-2005 9.7.2). With SystemVerilog
   generations neither fires, which matches 1800 (initialization without an event).
   Test: `tests/verilog/init2005.v`.
2. **`?:` with an unknown condition keeps z.** `s ? 1'bz : 1'bz` with `s = x` gives z; the
   combining table (1364-2005 5.1.13, table 5-21) gives x for z/z. ivtest `pr2172606b` says so
   in a comment ("technically incorrect for 1'bz inputs"). Test: `tests/verilog/condblend.v`.
3. **Event-controlled nonblocking assignments update at the trigger.** `x <= @(e) x + 1;`
   followed by `->e;` shows the new `x` in the triggering thread straight away (ivtest
   `nb_ec_multi_ev`). The NBA should be scheduled in the NBA region after the event. Not
   matched by ferrooxide yet.
4. **`logic` is reserved with `-g2005`.** `reg logic;` is a syntax error, but `logic` is not a
   1364-2005 keyword (`bit`, `int`, `byte`, `do`, `priority`, ... are accepted as names).
5. **`$urandom(seed)` leaks its seed into the unseeded generator.** After `$urandom(s)` the
   next `$urandom` / `$urandom_range` without a seed continues from the updated `s` (they
   share one internal seed); `$random(s)` does not do this. Test: `tests/verilog/random.v`.
7. **A real converted to a 2-state integer keeps x.** `int i = 1.0/0.0;` (or NaN) leaves `i`
   x, although `int` is 2-state (1800-2017 6.11: 2-state types hold only 0 and 1; the
   conversion of inf/NaN is undefined, and a 2-state result should be 0 or 1). Gates fed by
   the same real give x too. ivtest `br_gh1223a`. Test: `tests/verilog/twostate.v`.
