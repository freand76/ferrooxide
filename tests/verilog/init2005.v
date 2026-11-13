// iverilog: -g2005
// Verilog-2005: variable initializers are time-0 assignments with change events (not edges)
module top;
  reg c1 = 1'b1;
  reg [3:0] a = 4'd3;
  always @(posedge c1) $display("%0t posedge c1", $time);
  always @(c1) $display("%0t any c1", $time);
  always @(a) $display("%0t any a", $time);
  initial begin $display("%0t init", $time); #0 $display("%0t init #0", $time); end
endmodule
