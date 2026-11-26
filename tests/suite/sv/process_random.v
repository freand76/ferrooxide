// Random stability (IEEE 1800-2017 18.13, 18.14): each process has its own random number
// generator, a fork branch's seeded from its parent's; process::srandom,
// get_randstate and set_randstate; $urandom(seed) leaves the process's generator alone.
module t;
  integer errors = 0;
  int a[4], b[4], c[4];
  int x, y, s;
  string st;
  process p;
  task check(input integer line, input bit ok);
    if (!ok) begin
      $display("FAILED line %0d", line);
      errors = errors + 1;
    end
  endtask
  // the sequence of a branch does not depend on what another branch draws
  task run(input int extra, output int r[4]);
    fork
      begin
        repeat (extra) void'($urandom);
      end
      begin
        #1;
        foreach (r[i]) r[i] = $urandom;
      end
    join
  endtask
  initial begin
    p = process::self();
    st = p.get_randstate();
    foreach (a[i]) a[i] = $urandom;
    p.set_randstate(st);
    foreach (b[i]) b[i] = $urandom;
    check(`__LINE__, a == b && a[0] != a[1]);
    p.srandom(5);
    x = $urandom_range(1000);
    p.srandom(5);
    y = $urandom_range(1000);
    check(`__LINE__, x == y);
    // a seeded call does not move the process's generator
    st = p.get_randstate();
    s = 3;
    void'($urandom(s));
    check(`__LINE__, p.get_randstate() == st);
    // two forks from the same parent state give the same branch sequences
    st = p.get_randstate();
    run(0, a);
    p.set_randstate(st);
    run(5, b);
    check(`__LINE__, a == b);
    // a fork from another parent state gives others
    run(0, c);
    check(`__LINE__, c != a);
    if (errors == 0) $display("PASSED");
  end
endmodule
