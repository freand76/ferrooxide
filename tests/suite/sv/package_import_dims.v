// A type declared in a package keeps referring to the parameters its dimensions name,
// also those the package imported from another one, wherever the type is used
// (IEEE 1800-2017 26.3).
package A;
  localparam N = 4;
  localparam K = 3;
endpackage

package B;
  import A::*;
  import A::K;
  localparam M = N * 2;
  typedef logic [N-1:0] t_n;
  typedef struct packed { logic [M-1:0] x; t_n y; logic [K:0] z; } s_t;
  typedef logic [N:0] arr_t [K];
endpackage

module test;
  import B::*;
  localparam N = 100;
  s_t v;
  t_n w;
  arr_t a;
  initial begin
    if ($bits(v) == 16 && $bits(w) == 4 && $bits(a[0]) == 5 && $size(a) == 3) $display("PASSED");
    else $display("FAILED");
  end
endmodule
