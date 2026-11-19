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
   `nb_ec_multi_ev`). The NBA should be scheduled in the NBA region after the event, as
   ferrooxide does (a thread per pending update; the value and the target's indices are
   taken at the statement). Test: `tests/verilog/nba_event_control.v`.
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
10. **Interface port arrays connect lowest index to lowest index.** A port `I p[1:0]`
   connected to `I y[2:3]()` gets `p[0]` = `y[2]` and `p[1]` = `y[3]`, whatever the
   directions; 1800-2017 23.3.3 (connections of unpacked arrays, as unpacked array
   assignment, 7.6) pairs the elements from the left (`p[1]` = `y[2]`). Same directions
   give the same result. Test: `tests/verilog/sv_iface_array.v`.
11. **Immediate `cover` statements do nothing.** `cover (i == 1) $display("covered");` never
   runs its statement and prints nothing, at compile time or at run time; 1800-2017 16.3 runs
   the pass statement when the expression is true. Test: `tests/verilog/sv_assertions.v`.
12. **Array queries of a parameter ignore its declared range.** For `localparam [3:10] P = 0;`
   `$left(P)` is 7 and `$increment(P)` 1 (as for `[7:0]`), and `$size(Q + 1'b1)` of an
   expression is x; 1800-2017 20.7 gives the declared range (3, -1) and the expression's
   width. ferrooxide follows the standard here. Test: `tests/suite/sv/array_query_param.v`.
13. **An index wider than 32 bits counts by its low 32 bits.** With `reg [127:0] b = 2**64 + 7;`
   `array[b]`, `v[b]` and `v[b +: 4]` use index 7 (the low 32 bits as a signed integer: with
   bit 31 set the index is negative, out of range); an x/z bit anywhere still gives x, and a
   constant index is not cut. 1364-2005 5.2.1 makes such an index out of range (x on read,
   no write). ivtest `signed_a`. Test: `tests/verilog/wide_index.v`.
14. **A parameter with a string literal value prints as text.** `parameter P = "PASSED";
   $display(P);` prints `PASSED`, also with a range (`parameter [47:0] P = "..."`), for a
   concatenation of literals, a parameter set from such a parameter and a string override;
   1364-2005 3.6 / 1800-2017 5.9 make a string literal a vector, which `$display` prints as
   a decimal number. Only a bare parameter name as an argument (`P + 0` is a number).
   ivtest `param_string`. Test: `tests/verilog/param_string_display.v`.
15. **`shortreal` is a 64-bit `real`.** `shortreal x = 0.1; $display("%.20f", x);` prints
   `0.10000000000000000555` (a double); 1800-2017 6.12 makes `shortreal` the C `float`
   (32 bits), which would print `0.10000000149011611938`. ivtest `task_port_types1`.
   Test: `tests/verilog/shortreal.v`.
