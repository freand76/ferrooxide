// Subroutines that are never called (with timing controls, outputs, recursion) do not
// change the simulation.
module test;
  reg clk = 0;
  integer n = 0;
  task automatic wait_clk(output integer k);
    @(posedge clk) k = n;
    #1;
  endtask
  function automatic integer fact(input integer x);
    fact = x <= 1 ? 1 : x * fact(x - 1);
  endfunction
  initial begin
    n = 4;
    #1 if (n == 4) $display("PASSED");
    else $display("FAILED");
  end
endmodule
