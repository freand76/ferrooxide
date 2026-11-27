// Method tasks that wait, called from several processes at once, recursively and through
// each other: each call keeps its own automatic variables, arguments and `this` while it
// waits (IEEE 1800-2017 8.6, 13.3, 13.4.2), and returns its output arguments at its end.
module top;
  reg clk = 0;
  always #5 clk = ~clk;
  int errors = 0;
  event go;
  int level = 0;

  class Worker;
    int id;
    int done_at;
    function new(int i);
      id = i;
    endfunction
    // waits on a delay, a clock edge and a condition; a local variable lives across them
    task run(input int n, output int sum);
      int acc = 0;
      for (int k = 0; k < n; k++) begin
        int v = id * 10 + k;
        #1;
        @(posedge clk);
        acc += v;
      end
      wait (level >= id);
      done_at = $time;
      sum = acc;
    endtask
    // recursion that waits at each level
    task automatic count(input int n, inout int seen);
      int mine = n;
      if (n > 0) begin
        #2;
        count(n - 1, seen);
      end
      if (mine != n) errors++;
      seen = seen * 2 + mine;
    endtask
    // a method task calling another one that waits; a result through `this`
    task twice(output int a, output int b);
      run(1, a);
      run(2, b);
    endtask
    task wait_go();
      @go;
      done_at = $time;
    endtask
  endclass

  Worker w1, w2, w3;
  int s1, s2, s3, c1, c2, t1, t2;

  initial begin
    w1 = new(1);
    w2 = new(2);
    w3 = new(3);
    fork
      w1.run(3, s1);
      w2.run(2, s2);
      begin
        c1 = 0;
        w1.count(4, c1);
      end
      begin
        c2 = 1;
        w2.count(2, c2);
      end
      w3.twice(t1, t2);
      begin
        #40 level = 3;
      end
    join
    // run: the sum of id*10+k over k < n
    if (s1 != 10 + 11 + 12) begin
      $display("FAIL s1 %0d", s1);
      errors++;
    end
    if (s2 != 20 + 21) begin
      $display("FAIL s2 %0d", s2);
      errors++;
    end
    if (t1 != 30 || t2 != 30 + 31) begin
      $display("FAIL twice %0d %0d", t1, t2);
      errors++;
    end
    // count(n): seen = seen*2 + n after the inner calls, from n = 0 up
    if (c1 != ((((0 * 2 + 0) * 2 + 1) * 2 + 2) * 2 + 3) * 2 + 4) begin
      $display("FAIL c1 %0d", c1);
      errors++;
    end
    if (c2 != ((1 * 2 + 0) * 2 + 1) * 2 + 2) begin
      $display("FAIL c2 %0d", c2);
      errors++;
    end
    if (w1.done_at != 40 || w2.done_at != 40 || w3.done_at <= 40) begin
      $display("FAIL done_at %0d %0d %0d", w1.done_at, w2.done_at, w3.done_at);
      errors++;
    end
    // a named event, two waiters in the same task at once
    fork
      w1.wait_go();
      w2.wait_go();
      #7 ->go;
    join
    if (w1.done_at != $time || w2.done_at != $time) begin
      $display("FAIL go %0d %0d at %0d", w1.done_at, w2.done_at, $time);
      errors++;
    end
    if (errors == 0) $display("PASSED");
    $finish;
  end
endmodule
