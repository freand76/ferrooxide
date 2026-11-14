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
8. **`~`, `not` and `buf` on an input that never changes stay z.** `wire b; wire w = ~b;
   buf(a, b);` leaves `w` and `a` z (an undriven input is never evaluated), although `~z` and
   `buf` of z are x (1364-2005 5.1, table 7-7); `b & b` and `and` gates give x. Not matched by
   ferrooxide yet (it evaluates every continuous assignment at time 0). ivtest `sv_unit1c`.
9. **Array typedef dimensions come first.** `typedef logic [7:0] T1[3]; T1 x[5];` gives `x`
   the dimensions [3][5] (`$size(x, 1)` is 3, `x[4][2]` is out of bounds); 1800-2017 7.4.5
   (`mem_type ar [0:7]`) makes the declaration's own dimension the outer one ([5][3]).
   ivtest `sv_typedef_nested_array`. Test: `tests/verilog/typedef_unpacked.v`.
