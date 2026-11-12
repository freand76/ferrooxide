// Scheduling order that testbenches rely on (matches Icarus's FIFO event order):
// - always @* reaches its wait before initial blocks change its inputs
// - data changed before the clock in the same step: the flop sees the new combinational value
// - clock changed before the data in the same step: the flop sees the old value
module tb;
  reg clk = 0;
  reg [7:0] d1, d2;
  wire [7:0] c1 = d1 + 1, c2 = d2 + 1;
  reg [7:0] p, q1, q2;
  always @* p = d1 * 2;
  always @(posedge clk) begin
    q1 <= c1;
    q2 <= c2;
  end
  initial begin
    d1 = 1; d2 = 1;
    #1 $display("p=%0d c1=%0d", p, c1);
    d1 = 10;       // data first
    clk = 1;
    d2 = 20;       // data after the clock
    #1 $display("q1=%0d q2=%0d p=%0d", q1, q2, p);
    clk = 0;
    #1 d1 = 3; clk = 1; d2 = 4;
    #1 $display("q1=%0d q2=%0d p=%0d", q1, q2, p);
  end
endmodule
