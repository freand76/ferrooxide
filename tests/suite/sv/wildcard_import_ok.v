// A name in two wildcard-imported packages is fine when the module does not use it, or
// declares it before the use; locals of blocks and subroutines hide the imported names.
package p1;
  integer x = 1;
  integer y = 10;
  integer z = 100;
endpackage

package p2;
  integer x = 2;
  integer y = 20;
endpackage

module test;
  import p1::*;
  import p2::*;
  integer y = 5;
  function integer f(input integer x);
    f = x + 1;
  endfunction
  initial begin
    integer x;
    x = 7;
    if (y === 5 && x === 7 && f(3) === 4 && z === 100 && p2::x === 2)
      $display("PASSED");
    else
      $display("FAILED");
  end
endmodule
