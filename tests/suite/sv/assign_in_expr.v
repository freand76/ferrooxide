// expected
// assignments in expressions (IEEE 1800-2017 11.3.6): blocking, the value is the target's
// new value (of its type)
module top;
  int a, b, c, n;
  real r, q;
  logic [7:0] v;
  byte by;
  string s, t;
  int arr [4];
  initial begin
    a = (b = (c = 5));
    $display("1: %0d %0d %0d", a, b, c);
    q = (r = 2.5) * 2;
    $display("2: %f %f", r, q);
    if ((a -= 5)) $display("bad"); else $display("3: %0d", a);
    n = 0;
    while ((a += 1) < 4) n++;
    $display("4: %0d %0d", a, n);
    by = 8'sd100;
    v = (by += 100);
    $display("5: %0d %0d", by, v);
    arr[(c = 2)] = (b = 7);
    $display("6: %0d %0d %0d", arr[2], c, b);
    s = (t = "hi");
    $display("7: %s %s", s, t);
  end
endmodule
