// expected
// Named sequences and properties with arguments (16.8, 16.12), the default clocking and
// `default disable iff` (16.15): a reset between ticks ends the running attempts (the one
// of 45), no attempt starts while it holds; $past and $stable go on through such ticks.
module t;
  reg clk = 0, rst = 1;
  reg [3:0] x = 0;
  reg req = 0, ack = 0;
  always #5 clk = ~clk;
  sequence handshake(r, g, n);
    r ##[1:n] g;
  endsequence
  property ok(r, g);
    r |-> handshake(r, g, 2);
  endproperty
  default clocking cb @(posedge clk); endclocking
  default disable iff (rst);
  a_ok: assert property (ok(req, ack)) else $display("%0t a_ok fail", $time);
  a_past: assert property ($past(x, 2) == x - 2) else $display("%0t a_past fail x=%0d", $time, x);
  a_st: assert property (@(posedge clk) disable iff (0) $stable(x) || $changed(x)) else $display("%0t never", $time);
  c_st: cover property ($stable(x)) $display("%0t stable", $time);
  always @(negedge clk) if (!rst) x <= x + 1;
  initial begin
    #12 rst = 0;
    @(negedge clk) req = 1;            // 20: ack at 35 -> pass
    @(negedge clk) req = 0; ack = 1;   // 30
    @(negedge clk) ack = 0; req = 1;   // 40: req at 45, no ack at 55, 65
    @(negedge clk) req = 0;            // 50
    #3 rst = 1;                        // 53: disables the attempt of 45
    #4 rst = 0;                        // 57
    @(negedge clk) req = 1;            // 60: attempt at 65, fails at 85
    @(negedge clk) req = 0;
    #40 $finish;
  end
endmodule
