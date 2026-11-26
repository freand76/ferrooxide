// $sampled outside a concurrent assertion (IEEE 1800-2017 16.9.3): the value in the
// Preponed region of the time step, also in an action block
module top;
  logic clk = 0, a = 0, b = 0;
  logic [3:0] v = 4'd3;
  int fails = 0, fired = 0;
  always #5 clk = ~clk;
  initial begin
    #2;
    v = 4'd9;
    if ($sampled(v) !== 4'd3 || $sampled(v + 1) !== 4'd4 || v !== 4'd9) fails++;
    #1;
    if ($sampled(v) !== 4'd9) fails++;
  end
  // b follows a one cycle late: the assertion fails at 15 with b sampled as 0
  always @(posedge clk) b <= a;
  initial begin
    @(negedge clk) a = 1;
  end
  check: assert property (@(posedge clk) a |-> b)
    else begin
      fired++;
      if ($sampled(b) !== 1'b0 || $sampled(a) !== 1'b1) fails++;
    end
  initial begin
    #40;
    if (fails == 0 && fired == 1) $display("PASSED");
    else $display("FAILED %0d %0d", fails, fired);
    $finish;
  end
endmodule
