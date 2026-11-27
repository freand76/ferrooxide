// A process that waits inside a method task and is disabled or killed abandons the call
// (IEEE 1800-2017 9.6.2, 9.7); calling the task again starts a new call. Another call of
// the same task that waits meanwhile keeps its own variables.
module top;
  int errors = 0;

  class Pinger;
    int ticks;
    task tick(input int n);
      int left = n;
      while (left > 0) begin
        #2;
        left--;
        ticks++;
      end
    endtask
  endclass

  Pinger a, b, c;
  process pk;

  initial begin
    a = new;
    b = new;
    c = new;
    fork
      begin : stopped
        forever a.tick(5);
      end
      b.tick(6);
      begin
        #5 disable stopped;
      end
    join
    // a ticked twice before it was disabled; b's call went on alone
    if (a.ticks != 2 || b.ticks != 6) begin
      $display("FAIL disable %0d %0d", a.ticks, b.ticks);
      errors++;
    end
    // the disabled call does not continue: the same call, made again by the same process,
    // counts from its own argument
    a.ticks = 0;
    fork
      repeat (2) begin : again
        a.tick(5);
      end
      #5 disable again;
    join
    if (a.ticks != 2 + 5) begin
      $display("FAIL again %0d", a.ticks);
      errors++;
    end
    fork
      begin
        pk = process::self();
        c.tick(10);
      end
      begin
        #5 pk.kill();
      end
    join
    #20;
    if (c.ticks != 2) begin
      $display("FAIL kill %0d", c.ticks);
      errors++;
    end
    c.ticks = 0;
    c.tick(1);
    if (c.ticks != 1) begin
      $display("FAIL after kill %0d", c.ticks);
      errors++;
    end
    if (errors == 0) $display("PASSED");
    $finish;
  end
endmodule
