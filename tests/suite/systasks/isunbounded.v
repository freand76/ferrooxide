// A parameter may be `$` (IEEE 1800-2017 6.20.2): $isunbounded (20.6.3) is true for it and
// for `$`, and in a cycle range it stands for `$`.
module t #(parameter int MAX = $, parameter int N = 3);
  localparam bit U = $isunbounded(MAX);
  reg clk = 0, req = 0, ack = 0;
  integer fails = 0, fails_n = 0;
  always #5 clk = ~clk;
  // req is followed by ack, however late
  assert property (@(posedge clk) $rose(req) |-> ##[1:MAX] ack) else fails++;
  // (within N cycles: fails)
  assert property (@(posedge clk) $rose(req) |-> ##[1:N] ack) else fails_n++;
  initial begin
    if (!U || $isunbounded(N) || !$isunbounded($) || $isunbounded(1)) $display("FAILED: isunbounded");
    @(negedge clk) req = 1;
    repeat (20) @(negedge clk);
    ack = 1;
    @(negedge clk) ack = 0;
    #20;
    if (fails == 0 && fails_n == 1) $display("PASSED");
    else $display("FAILED: %0d, %0d fails", fails, fails_n);
    $finish;
  end
endmodule
