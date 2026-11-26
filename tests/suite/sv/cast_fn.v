// $cast as a function and as a task (IEEE 1800-2017 6.24.2, 8.16): an enum target takes
// only its constants' values, a class handle null or an object of its class or a derived
// one; any other target takes the value converted. A failed cast leaves the target as it is.
module top;
  typedef enum logic [2:0] {R = 1, G = 2, B = 4} col_t;
  class Base;
    int v = 1;
  endclass
  class Der extends Base;
    int w = 2;
  endclass
  class Other extends Base;
  endclass
  int a;
  col_t c;
  Base b;
  Der d;
  int ok;
  int fails;
  initial begin
    fails = 0;
    // a real converted to int
    if (!$cast(a, 2.1 * 3.7)) fails++;
    if (a != 8) fails++;
    // enum values
    c = R;
    if (!$cast(c, 3'd4) || c != B) fails++;
    ok = $cast(c, 3);
    if (ok != 0 || c != B) fails++;
    $cast(c, 2);
    if (c != G) fails++;
    // downcasts
    d = new;
    d.w = 7;
    b = d;
    d = null;
    if (!$cast(d, b) || d.w != 7) fails++;
    begin
      Other o = new;
      b = o;
    end
    d = null;
    if ($cast(d, b) || d != null) fails++;
    b = null;
    if (!$cast(d, b) || d != null) fails++;
    // an upcast
    d = new;
    if (!$cast(b, d) || b.v != 1) fails++;
    if (fails == 0)
      $display("PASSED");
    else
      $display("FAILED %0d", fails);
  end
endmodule
