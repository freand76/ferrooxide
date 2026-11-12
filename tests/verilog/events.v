// Named events: several waiters, hierarchical triggers, triggers from tasks and fork branches,
// a trigger nobody waits for, and @(a or ev).
module sub;
  event go;
  integer seen = 0;
  always @(go) begin
    seen = seen + 1;
    $display("%0t: sub saw go (%0d)", $time, seen);
  end
endmodule

module tb;
  event e1, e2;
  reg a = 0;
  sub s();
  task fire;
    -> e2;
  endtask
  initial begin
    @(e1) $display("%0t: waiter 1 got e1", $time);
    @(e1 or a) $display("%0t: waiter 1 got e1 or a", $time);
  end
  initial begin
    @(e1) $display("%0t: waiter 2 got e1", $time);
  end
  always @(e2) $display("%0t: e2", $time);
  initial begin
    -> e1;          // at time 0 the waiters may not be waiting yet
    #5 -> e1;
    #5 a = 1;
    #5 fire;
    #5 -> s.go;
    #1 -> s.go; -> s.go;
    fork
      #2 -> e2;
      #3 -> s.go;
    join
    #1 $display("%0t: seen=%0d", $time, s.seen);
  end
endmodule
