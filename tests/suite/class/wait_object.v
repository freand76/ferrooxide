// `wait` on a condition that reads objects wakes when the object changes.
module test;
  class C;
    int q[$];
    int n;
    task get(output int v);
      wait (q.size() > 0);
      v = q.pop_front();
    endtask
  endclass
  C c;
  int r;
  int t;
  initial begin
    c = new;
    fork
      begin
        c.get(r);
        t = $time;
      end
      begin
        #2 c.n = 1;
        #3 c.q.push_back(7);
      end
      begin
        wait (c.n == 1);
        if ($time != 2) $display("FAILED: wait on a property at %0t", $time);
      end
    join
    if (r == 7 && t == 5) $display("PASSED");
    else $display("FAILED: %0d at %0d", r, t);
  end
endmodule
