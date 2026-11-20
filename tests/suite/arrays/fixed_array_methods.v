// Array manipulation methods of fixed-size arrays (IEEE 1800-2017 7.12) and assignments
// between fixed-size and dynamic arrays (7.6: elements pair up from left to right).
class C;
  int v;
  function new(int x);
    v = x;
  endfunction
endclass

module test;
  int f[4] = '{3, 1, 2, 5};
  int g[3:0];
  byte b[2:0] = '{1, 2, 3};
  string s[3] = '{"bb", "a", "ccc"};
  real r[2] = '{2.5, 1.5};
  C cs[3];
  int q[$];
  int wakes = 0;
  int fails = 0;

  task check(input bit ok, input string what);
    if (!ok) begin
      $display("FAILED: %s", what);
      fails++;
    end
  endtask

  always @(f[0]) wakes++;

  initial begin
    #1;
    // reductions and locators
    check(f.sum == 11 && f.product == 30 && f.or == 7, "reductions");
    check(b.sum with (int'(item)) == 6, "sum with");
    q = f.find_index with (item > 2);
    check(q.size == 2 && q[0] == 0 && q[1] == 3, "find_index");
    q = f.max;
    check(q.size == 1 && q[0] == 5, "max");
    q = b.find_first_index with (item == 3);
    check(q.size == 1 && q[0] == 0, "find_first_index of a descending range");
    // fixed-size to dynamic and back
    q = f;
    check(q.size == 4 && q[0] == 3 && q[3] == 5, "queue from a fixed-size array");
    g = f;
    check(g[3] == 3 && g[2] == 1 && g[1] == 2 && g[0] == 5, "descending array from an ascending one");
    q = '{9, 8, 7, 6};
    g = q;
    check(g[3] == 9 && g[0] == 6, "fixed-size array from a queue");
    // ordering (writes wake waiters on the elements)
    wakes = 0;
    f.sort;
    #1;
    check(f[0] == 1 && f[1] == 2 && f[2] == 3 && f[3] == 5, "sort");
    check(wakes == 1, "sort wakes a waiter on an element");
    f.rsort with (item % 3);
    check(f[0] == 2 && f[1] == 5 && f[2] == 1 && f[3] == 3, "rsort with");
    g.reverse;
    check(g[3] == 6 && g[0] == 9, "reverse of a descending array");
    s.sort;
    check(s[0] == "a" && s[1] == "bb" && s[2] == "ccc", "sort of strings");
    r.rsort;
    check(r[0] == 2.5 && r[1] == 1.5, "rsort of reals");
    cs[0] = new(4);
    cs[1] = new(9);
    cs[2] = new(2);
    check(cs.sum with (item.v) == 15, "sum of class properties");
    cs.sort with (item.v);
    check(cs[0].v == 2 && cs[1].v == 4 && cs[2].v == 9, "sort of handles");
    // a nonblocking assignment from a queue
    q = '{1, 2, 3, 4};
    g <= q;
    check(g[3] == 6, "nonblocking: not yet");
    #1;
    check(g[3] == 1 && g[0] == 4, "nonblocking: applied");
    if (fails == 0) $display("PASSED");
  end
endmodule
