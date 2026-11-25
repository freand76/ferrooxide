// expected
// A call of a function with another call of the same function in a later argument: all
// arguments are evaluated before the (shared) port variables are assigned.
module top;
  function [15:0] add(input [15:0] a, input [15:0] b);
    add = a + b;
  endfunction
  function [15:0] sub3(input [15:0] a, input [15:0] b, input [15:0] c);
    sub3 = a - b - c;
  endfunction
  function automatic [15:0] aadd(input [15:0] a, input [15:0] b);
    aadd = a + b;
  endfunction
  task t(input [15:0] a, input [15:0] b, output [15:0] s);
    s = a * 10 + b;
  endtask
  reg [15:0] r;
  initial begin
    $display("%0d", add(1, add(4, 2)));
    $display("%0d", add(add(1, 2), add(add(3, 4), 5)));
    $display("%0d", sub3(100, sub3(50, 1, 2), add(3, 4)));
    $display("%0d", aadd(1, aadd(4, 2)));
    t(add(1, 2), add(3, 4), r);
    $display("%0d", r);
  end
endmodule
