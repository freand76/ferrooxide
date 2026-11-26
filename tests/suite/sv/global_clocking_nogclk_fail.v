// exit: 1
// a global clocking sampled value function needs a global clocking block (IEEE 1800-2017
// 16.9.4)
module top;
  logic a, clk;
  assert property (@(posedge clk) $rose_gclk(a));
endmodule
