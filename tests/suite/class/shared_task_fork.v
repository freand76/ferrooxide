// Method tasks with forks, called from several processes at once and recursively from
// their own branches: each branch sees the automatic variables, arguments and `this` of
// the call that started it (IEEE 1800-2017 9.3.2, 13.3), also after a join_none call
// has returned; `wait fork` and `disable fork` act on the calling process's branches
// (9.6.1, 9.6.3).
module top;
  int errors = 0;
  int log[$];

  class Seq;
    int id;
    int state;
    Seq child;
    function new(int i);
      id = i;
    endfunction
    // as a sequence's start: the body runs in a branch the call joins
    task start(input int depth, output int res);
      int mine = id * 100 + depth;
      state = 1;
      fork
        begin
          #1;
          state = 2;
          if (depth > 0) begin
            int r;
            child = new(id + 1);
            child.start(depth - 1, r);
            mine += r;
          end
          #1;
          if (mine / 100 < id) errors++;
          state = 3;
        end
      join
      res = mine;
    endtask
    // branches that outlive the call read its arguments and locals later
    task spawn(input int n, input int d);
      int k = n * 10;
      fork
        begin
          #d;
          log.push_back(k + n);
        end
        begin
          #(d + 1);
          log.push_back(-(k + n));
        end
      join_none
    endtask
    // a fork block's automatic variable: a copy per branch
    task loop(input int n);
      for (int i = 0; i < n; i++)
        fork
          automatic int j = i;
          begin
            #(j + 1);
            log.push_back(1000 + j);
          end
        join_none
      wait fork;
    endtask
    // the first branch to end wins, the others are disabled
    task race(input int a, input int b, output int win);
      fork
        begin #a; win = 1; end
        begin #b; win = 2; end
      join_any
      disable fork;
    endtask
  endclass

  Seq s1, s2;
  int r1, r2, w1, w2;
  initial begin
    s1 = new(1);
    s2 = new(5);
    fork
      s1.start(2, r1);
      s2.start(1, r2);
    join
    // s1: 102 + (201 + 300) ; s2: 501 + 600
    if (r1 != 603 || r2 != 1101) begin errors++; $display("start %0d %0d", r1, r2); end
    if (s1.state != 3 || s1.child.state != 3 || s1.child.child.state != 3) errors++;

    log.delete();
    s1.spawn(1, 3);
    s1.spawn(2, 1);
    #5;
    if (log.size() != 4 || log[0] != 22 || log[1] != -22 || log[2] != 11 || log[3] != -11) begin errors++; $display("spawn %p", log); end

    log.delete();
    fork
      s1.loop(3);
      s2.loop(2);
    join
    if (log.size() != 5 || $time != 14) begin errors++; $display("loop %p at %0t", log, $time); end

    fork
      s1.race(3, 1, w1);
      s2.race(1, 4, w2);
    join
    if (w1 != 2 || w2 != 1 || $time != 15) begin errors++; $display("race %0d %0d at %0t", w1, w2, $time); end
    #10;
    if (w1 != 2 || w2 != 1) begin errors++; $display("race after %0d %0d", w1, w2); end

    if (errors == 0) $display("PASSED");
    else $display("FAILED: %0d errors", errors);
  end
endmodule
