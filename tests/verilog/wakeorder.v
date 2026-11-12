// Order in which processes woken by the same change run (Icarus: newest waiter first).
module tb;
  reg clk = 0;
  always @(posedge clk) $display("%0t flop 1", $time);
  always @(posedge clk) $display("%0t flop 2", $time);
  always @(posedge clk) $display("%0t flop 3", $time);
  initial repeat (3) begin #1 clk = 1; #1 clk = 0; end
endmodule
