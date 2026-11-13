`timescale 1ns/1ns
module counter (input clk, input rst, output reg [7:0] q);
  always @(posedge clk) if (rst) q <= 0; else q <= q + 1;
endmodule
module top;
  reg clk = 0, rst = 1;
  wire [7:0] q;
  wire msb = q[7];
  counter u1 (clk, rst, q);
  always #5 clk = ~clk;
  initial #12 rst = 0;
  initial #1000 $finish;
endmodule
