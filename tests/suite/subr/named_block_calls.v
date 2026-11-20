// A function with a named block called from several places compiles the block for each
// call; that is not a second declaration of its name.
module test;
  function integer f(input integer x);
    begin : body
      integer y;
      y = x + 1;
      f = y;
    end
  endfunction
  integer a, b;
  initial begin
    a = f(1);
    b = f(2);
  end
  initial #1
    if (a === 2 && b === 3 && f(3) === 4) $display("PASSED");
    else $display("FAILED");
endmodule
