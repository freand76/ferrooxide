// %p (IEEE 1800-2017 21.2.1.7): values as elements of an assignment pattern, arrays as
// assignment patterns (queues, dynamic arrays, fixed-size arrays, a locator's result).
module test;
  int i = -5;
  logic [7:0] v = 8'h1x;
  int q[$] = '{1, 2, 3};
  int d[] = '{-1, 7};
  real rq[$] = '{1.5, 2.0};
  string sq[$] = '{"a", "bc"};
  int f[3] = '{4, 5, 6};
  byte g[2:0] = '{1, 2, 3};
  string s = "hi";
  real r = 2.5;
  int e[$];
  int fails = 0;

  task check(input string got, input string want);
    if (got != want) begin
      $display("FAILED: '%s', expected '%s'", got, want);
      fails++;
    end
  endtask

  initial begin
    check($sformatf("%p|%p|%p|%p", i, v, s, r), "-5|X|\"hi\"|2.5");
    check($sformatf("%p", q), "'{1, 2, 3}");
    check($sformatf("%p", d), "'{-1, 7}");
    check($sformatf("%p", rq), "'{1.5, 2}");
    check($sformatf("%p", sq), "'{\"a\", \"bc\"}");
    check($sformatf("%p", f), "'{4, 5, 6}");
    check($sformatf("%p", g), "'{1, 2, 3}");
    check($sformatf("%p", e), "'{}");
    check($sformatf("%p", q.find with (item > 1)), "'{2, 3}");
    check($sformatf("x%px", "ab"), "x\"ab\"x");
    if (fails == 0) $display("PASSED");
  end
endmodule
