// nonblocking event triggers (IEEE 1800-2017 15.5.1): the event is triggered in the NBA
// region, after the waits of the time step began and with the NBA updates done
module inner;
  initial #3 ->> top.e;
endmodule

module top;
  event e;
  logic x = 0;
  int n = 0, seen = -1, fails = 0;
  inner i();
  always @e n++;
  initial begin
    #1;
    x <= 1;
    ->> e;
    // (the trigger comes after this wait starts)
    @e seen = x;
    #1;
    if (n != 1 || seen !== 1) fails++;
    // a blocking trigger comes before; the waiter misses it
    fork
      begin -> e; end
      begin #0 @e if ($time == 2) fails++; end
      begin #1; end
    join_any
    #3;
    // (and the one from another module at 3)
    if (n != 3) fails++;
    if (fails == 0) $display("PASSED");
    else $display("FAILED %0d %0d %0d", fails, n, seen);
  end
endmodule
