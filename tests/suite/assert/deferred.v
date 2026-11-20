// expected
// Deferred assertions (IEEE 1800-2017 16.4): `#0` reports in the Reactive region, `final` in
// the Postponed region, only for the last evaluation of the time step (an earlier report
// of the same assertion is flushed: the glitch at 10, q = 3 overwritten by an NBA at 30);
// outside a procedure one is in an always_comb of its own (16.4.3).
module t;
  reg a = 1, b = 0;
  reg [3:0] q = 0;
  wire c = a & ~b;
  // glitch: a and b change in the same step; c is 1, then 0, then 1 again
  assert #0 (c) else $display("%0t #0 fail c=%b", $time, c);
  assert final (c) $display("%0t final pass", $time); else $display("%0t final fail", $time);
  d1: cover #0 (q == 3) $display("%0t covered %m q=%0d", $time, q);
  always @(q) begin
    assert #0 (q != 5) else $display("%0t q is 5 (reported once)", $time);
  end
  initial begin
    #10 b = 1; a = 0; #0 b = 0; a = 1;   // the glitch resolves within the step
    #10 a = 0;                           // a real failure
    #10 a = 1;
    q = 3; q <= 5; #1 q = 6;
    #5 q = 3;
    #10 $finish;
  end
endmodule
