// expected
// A deferred assertion's report matures in the Observed (#0) or Postponed (final) region
// of its time step (IEEE 1800-2017 16.4); $finish in that time step ends the simulation
// before then (20.2), so its pending reports never appear.
module top;
  reg a = 0;
  initial begin
    assert final (a) else $display("%0t final failed", $time);
    #1;
    assert #0 (a) else $display("%0t #0 failed", $time);
    assert final (a) else $display("%0t final failed", $time);
    $finish;
  end
endmodule
