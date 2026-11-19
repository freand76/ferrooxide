// `import` in a begin-end block (IEEE 1800-2017 26.3): the package's names are visible in
// the block, over names of the enclosing module (a wildcard import hides only what the
// block itself declares); `import P::x` brings in one name; the names stay hidden outside.
// The sensitivity of always_comb / @* follows the imported names (9.2.2.2.1).
package P;
  typedef logic [3:0] word;
  parameter K = 7;
  int v = 1;
  event e;
  function int f(int a);
    return a + K;
  endfunction
  function int g(int a);
    return a * 2;
  endfunction
endpackage

module m;
  typedef logic [7:0] word;
  parameter K = 100;
  int v = 50;
  event e;
  function int f(int a);
    return a + K;
  endfunction
  function int g(int a);
    return a;
  endfunction
  int got = 0;
  int oc, os;
  always_comb begin
    import P::*;
    oc = v + 1;
  end
  always @* begin
    import P::v;
    os = v + 2;
  end
  initial begin : wild
    import P::*;
    word w;
    int K2;
    w = 8'hff;
    if ($bits(w) != 4 || w != 4'hf) $display("FAILED -- word %0d", $bits(w));
    if (K != 7 || f(1) != 8 || v != 1) $display("FAILED -- K %0d f %0d v %0d", K, f(1), v);
    v = 3;
    @e got = 1;
  end
  initial begin
    import P::g;
    if (g(5) != 10 || f(1) != 101 || K != 100) $display("FAILED -- explicit %0d %0d", g(5), f(1));
  end
  initial begin
    #1 -> P::e;
    #1;
    if ($bits(word) != 8 || v != 50 || P::v != 3 || got != 1) $display("FAILED -- outside %0d %0d %0d", v, P::v, got);
    P::v = 10;
    #1 if (oc != 11 || os != 12) $display("FAILED -- comb %0d %0d", oc, os);
    v = 60;
    #1 if (oc != 11 || os != 12) $display("FAILED -- comb woke on m.v %0d %0d", oc, os);
    $display("PASSED");
  end
endmodule
