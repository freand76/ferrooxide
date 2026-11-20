// Mailboxes and semaphores of the built-in package std (IEEE 1800-2017 15.3, 15.4):
// blocking and non-blocking methods, a bounded mailbox, FIFO order.
module test;
  mailbox #(string) m;
  mailbox #(int) b;
  semaphore s;
  string log = "";
  int fails = 0;

  task check(input bit ok, input string what);
    if (!ok) begin
      $display("FAILED: %s", what);
      fails++;
    end
  endtask

  initial begin
    string r, p;
    int v, i;
    m = new();
    check(m.num() == 0 && m.try_get(r) == 0 && m.try_peek(r) == 0, "empty");
    m.put("a");
    check(m.try_put("b") == 1, "try_put of an unbounded mailbox");
    m.peek(p);
    check(p == "a" && m.num() == 2, "peek leaves the message");
    m.get(r);
    check(r == "a" && m.num() == 1, "get in order");
    check(m.try_get(r) == 1 && r == "b", "try_get");
    // a bounded mailbox: put waits for room, get for a message
    b = new(1);
    s = new(1);
    fork
      begin
        b.put(1);
        check(b.try_put(9) == 0, "try_put of a full mailbox");
        b.put(2);
        log = {log, $sformatf("put2@%0t ", $time)};
      end
      begin
        #3 b.get(v);
        log = {log, $sformatf("get%0d@%0t ", v, $time)};
        b.get(v);
        log = {log, $sformatf("get%0d@%0t ", v, $time)};
      end
      begin
        s.get(1);
        #4 s.put(1);
      end
      begin
        #1;
        check(s.try_get(1) == 0, "try_get of a semaphore without keys");
        s.get(1);
        log = {log, $sformatf("sem@%0t ", $time)};
      end
    join
    check(log == "get1@3 put2@3 get2@3 sem@4 ", {"order: ", log});
    if (fails == 0) $display("PASSED");
  end
endmodule
