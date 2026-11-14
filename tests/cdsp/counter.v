`timescale 1ns/1ps
module sub(input clk, output reg [39:0] q);
  initial q = 0;
  always @(posedge clk) q <= q + 40'h1_0000_0001;
endmodule
module tb;
  reg clk = 0;
  reg [3:0] x = 4'bx01z;
  real r = 1.5;
  always #5 clk = ~clk;
  sub u(.clk(clk), .q());
  initial begin #22 x = 4'd9; #1000 $display("done at %t", $time); $finish; end
endmodule
