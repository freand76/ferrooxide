// expected
// Concurrent assertions (IEEE 1800-2017 16.14): expressions are sampled before the clock
// edge's time step (16.5.1), action blocks run in the Reactive region (after the edge's
// NBAs: p4 sees q = 7), the pass action also runs for a vacuous success, `cover` runs it for
// non-vacuous ones only, `%m` names the label. $fell at the first tick compares with the
// default value x (16.9.3).
module t;
  reg clk = 0;
  reg a = 0, b = 0, c = 0, rst = 0;
  reg [3:0] q = 0, d = 0;
  always #5 clk = ~clk;
  always @(posedge clk) q <= d;
  // overlapping / next-cycle implication
  p1: assert property (@(posedge clk) a |-> b) $display("%0t p1 pass", $time); else $display("%0t p1 fail %m", $time);
  p2: assert property (@(posedge clk) a |=> b) else $display("%0t p2 fail", $time);
  // ranges
  p3: assert property (@(posedge clk) a |-> ##[1:3] c) else $display("%0t p3 fail", $time);
  // action blocks see the NBA results of the tick
  p4: assert property (@(posedge clk) d != 4'd7) else $display("%0t p4 fail q=%0d", $time, q);
  // sampled value functions
  p5: assert property (@(posedge clk) $rose(a) |-> !$past(a)) else $display("%0t p5 fail", $time);
  c1: cover property (@(posedge clk) $fell(a)) $display("%0t c1 fell", $time);
  initial begin
    @(negedge clk) a = 1; b = 1;      // 10
    @(negedge clk) a = 0; b = 0;      // 20: p2 attempt from 15 fails at 25
    @(negedge clk) c = 1;             // 30
    @(negedge clk) a = 1; c = 0;      // 40: p1 fails at 45, p3 from 45 needs c at 55..75
    @(negedge clk) a = 0; d = 7;      // 50
    @(negedge clk) d = 0;             // 60
    #40 $finish;
  end
endmodule
