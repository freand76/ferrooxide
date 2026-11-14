// Verilog-2005 code with names that SystemVerilog reserves (lint: they do not compile as SV)
module interface(input clk, input [7:0] string, output reg [7:0] var);
  reg logic;
  integer class;
  always @(posedge clk) begin : unique
    var <= string;
    logic = ~logic;
  end
  task bit;
    class = class + 1;
  endtask
endmodule

module top;
  reg clk, ok;
  wire [7:0] q;
  interface do (.clk(clk), .string(8'h5), .var(q));
  initial begin
    ok = 1;   // an ordinary name: no finding
    clk = 0;
  end
endmodule
