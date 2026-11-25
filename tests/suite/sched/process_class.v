// expected
// The std::process class (IEEE 1800-2017 9.7): self() gives the same handle for a process,
// status() of the running, a waiting, a killed and a finished process, kill() of a fork
// branch with the branches it started (the parent's join counts it), await().
module t;
  process p1, p2, p3, me;
  initial begin
    me = process::self();
    $display("same %0d running %0d", me == process::self(), me.status() == process::RUNNING);
    fork
      begin p1 = process::self(); #10 $display("p1 ends at %0t", $time); end
      begin
        p2 = process::self();
        fork begin p3 = process::self(); #50 $display("p3 must not print"); end join_none
        #100 $display("p2 must not print");
      end
    join_none
    #1;
    $display("waiting %0d %0d", p1.status() == process::WAITING, p2.status() == process::WAITING);
    p2.kill();
    $display("killed %0d %0d", p2.status() == process::KILLED, p3.status() == process::KILLED);
    p1.await();
    $display("awaited at %0t: finished %0d", $time, p1.status == process::FINISHED);
    fork
      begin #5; end
      begin #200 $display("must not print"); end
      begin process q = process::self(); q.kill(); $display("must not print"); end
    join_any
    $display("join_any at %0t", $time);
    disable fork;
  end
endmodule
