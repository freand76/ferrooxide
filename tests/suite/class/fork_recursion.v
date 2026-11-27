// A subroutine that calls itself from a branch of its own fork (as a sequence's start()
// forks its body(), which starts child sequences). Each call's branches see that call's
// arguments and variables, and the caller keeps its own after the join.
class Seq;
  string name;
  int depth;
  Seq kids[$];
  static string log[$];
  function new(string n, int d); name = n; depth = d; endfunction
  virtual task body();
    if (depth > 0) begin
      Seq a = new({name, ".a"}, depth - 1), b = new({name, ".b"}, depth - 1);
      a.start(this);
      b.start(this);
    end
  endtask
  task start(Seq parent);
    string me = name;
    fork
      begin
        #1;
        body();
        log.push_back(me);
      end
    join
    // (the caller's own variables after the join)
    if (me != name || (parent != null && parent.depth != depth + 1)) log.push_back("BAD");
  endtask
endclass
module t;
  int got;
  // a function forking itself without waiting
  function automatic int twice(int k);
    if (k > 0)
      fork
        got = got + twice(k - 1);
      join_none
    return 2 * k;
  endfunction
  // a task whose branches wait while deeper calls run
  task automatic count(int k, output int n);
    int sub = 0;
    if (k == 0) begin
      n = 1;
      return;
    end
    fork
      begin
        int x;
        #1 count(k - 1, x);
        sub += x;
      end
      begin
        int y;
        #2 count(k - 1, y);
        sub += y;
      end
    join
    n = sub + k * 100;
  endtask
  Seq top;
  int bad = 0, n;
  initial begin
    if (twice(2) != 4) bad++;
    #0;
    if (got != 2) bad++;
    count(2, n);
    // count(1) = 1 + 1 + 100 = 102; count(2) = 102 + 102 + 200
    if (n != 404) bad++;
    top = new("t", 2);
    top.start(null);
    if (Seq::log.size() != 7 || Seq::log[6] != "t" || Seq::log[0] != "t.a.a") bad++;
    foreach (Seq::log[i]) if (Seq::log[i] == "BAD") bad++;
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %0d %0d %p", bad, n, Seq::log);
  end
endmodule
