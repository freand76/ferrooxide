// An event declared in a package, triggered and waited for by its scoped name and through
// a wildcard import (IEEE 1800-2017 15.5.1, 26.3).
package P;
  event e;
endpackage

module m;
  import P::*;
  int got = 0;
  initial begin
    #1 -> P::e;
    #1 -> e;
  end
  initial begin
    @(P::e) got += 1;
  end
  initial begin
    @e;
    @e got += 10;
  end
  initial begin
    #3 if (got != 11) $display("FAILED -- got %0d", got);
    $display("PASSED");
  end
endmodule
