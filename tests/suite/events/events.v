// Named events and event controls (IEEE 1800 9.4, 15.5): a trigger wakes only processes
// already waiting; @(a or b), @(a, b), @(posedge c iff en); `wait` on a level; repeat with
// an event control; .triggered (true for the rest of the time step after a trigger)
module t;
  integer errors = 0, hits = 0, iffs = 0, both = 0, late = 0;
  event e, e2;
  reg a, b, c, en;
  reg [3:0] n;
  always @e hits = hits + 1;
  always @(a or b) both = both + 1;
  always @(posedge c iff en) iffs = iffs + 1;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    a = 0; b = 0; c = 0; en = 0; n = 0;
    #1 -> e;
    #1 check(`__LINE__, hits, 1);
    a = 1; #1 b = 1; #1 a = 0;
    #1 check(`__LINE__, both, 4);          // also the x -> 0 changes at time 0
    c = 1; #1 c = 0; en = 1;
    #1 c = 1; #1 c = 0;
    #1 check(`__LINE__, iffs, 1);          // only while en is 1
    fork
      begin @e2; late = 1; end
      #0 -> e2;                            // after the waiter has started: woken
    join
    check(`__LINE__, late, 1);
    late = 0;
    -> e2;
    fork
      begin @e2; late = 1; end             // starts waiting after the trigger: not woken
      #2;
    join_any
    disable fork;
    check(`__LINE__, late, 0);
    fork
      repeat (3) @(posedge c) n = n + 1;
      begin #1 c = 1; #1 c = 0; #1 c = 1; #1 c = 0; #1 c = 1; end
    join
    check(`__LINE__, n, 3);
    fork
      begin
        -> e;
        wait (e.triggered);                // still set in this time step
        late = 2;
      end
    join
    check(`__LINE__, late, 2);
    if (errors == 0) $display("PASSED");
  end
endmodule
