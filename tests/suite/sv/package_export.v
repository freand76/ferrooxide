// Package exports (IEEE 1800-2017 26.6): a name a package imports and exports can be
// imported from it again, also through a chain of packages; `p::*` and `*::*` export the
// explicitly imported names and the wildcard-imported names the package refers to.
package base;
  localparam int W = 8;
  typedef logic [W-1:0] word_t;
  integer count = 5;
  function automatic int twice(int v);
    return 2 * v;
  endfunction
endpackage

package mid;
  import base::W;
  import base::*;
  export base::W;            // explicit import
  export base::word_t;       // wildcard import, exported by name
  export base::*;            // twice: referenced below
  localparam int T = twice(3);
endpackage

package top_pkg;
  import mid::*;
  import mid::word_t;
  export *::*;               // word_t
  word_t z = 8'h5a;
endpackage

module test;
  import mid::W;
  import mid::twice;
  import top_pkg::word_t;
  word_t a;
  initial begin
    a = 8'hff;
    if (W != 8 || $bits(a) != 8 || twice(4) != 8 || mid::T != 6 || mid::W != 8 || top_pkg::z != 8'h5a)
      $display("FAILED W=%0d bits=%0d twice=%0d", W, $bits(a), twice(4));
    else
      $display("PASSED");
  end
endmodule
