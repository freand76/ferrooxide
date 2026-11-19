// Parameters declared in tasks, functions and named blocks, used in declarations and
// expressions there; `parameter`s changed by defparam from outside; events triggered
// through indexed generate scopes.
module sub;
  function [7:0] scaled(input [7:0] x);
    parameter K = 3;
    localparam M = K * 2;
    scaled = x * M;
  endfunction

  task fill;
    parameter DEPTH = 4;
    parameter WIDTH = 6;
    reg [WIDTH-1:0] mem [0:DEPTH-1];
    integer i;
    begin
      for (i = 0; i < DEPTH; i = i + 1) mem[i] = i * 9;
      $display("fill %0d %0d %0d %0d", DEPTH, $bits(mem[0]), mem[DEPTH-1], scaled(2));
    end
  endtask

  genvar g;
  for (g = 0; g < 2; g = g + 1) begin : gb
    event e;
    always @e $display("gb[%0d] triggered at %0t", g, $time);
  end

  initial begin : blk
    parameter P = 1;
    localparam Q = P + 10;
    reg [Q-1:0] r;
    r = 0;
    $display("blk %0d %0d %0d", P, Q, $bits(r));
    fill;
  end
endmodule

module top;
  sub s();
  defparam s.blk.P = 5;
  defparam s.fill.DEPTH = 8;
  defparam s.scaled.K = 10;
  initial begin
    #1 -> s.gb[1].e;
    #1 -> s.gb[0].e;
  end
endmodule
