// SystemVerilog strings: assignment, compare, concat, methods, s[i], conversions, formats,
// change events, task arguments, $sformatf.
module t;
  string a, b, c;
  string arr [0:1];
  reg [8*4:1] r;
  integer n;
  real x;
  task automatic show(input string s, input integer k);
    $display("show %0s %0d", s, k);
  endtask
  always @(a) $display("a changed: %s", a);
  initial begin
    #1 a = "hello";
    #1 a = "hello";
    #1 a = "world";
    b = "abc";
    c = {a, "-", b};
    $display("%s %0d %s %s", c, c.len(), c.toupper(), c.substr(1, 3));
    $display("%b %b %b", a == "world", a < b, a != b);
    $display("%s %h", b[1], b[1]);
    b[0] = "X";
    $display("%s", b);
    r = "WXYZ";
    $display("%s", r);
    b = r;
    $display("%s %0d", b, b.len());
    n = 42;
    a.itoa(n); $display("%s", a);
    a.hextoa(-11); $display("%s", a);
    a = "123abc"; $display("%0d %0d", a.atoi(), a.atohex());
    a = "2.5x"; x = a.atoreal(); $display("%f", x);
    a = $sformatf("v=%0d/%s", n, b);
    $display(a);
    a = "%0s+%0s"; $display(a, "p", "q");
    arr[0] = "zero"; arr[1] = {arr[0], "one"};
    $display("%s %s", arr[0], arr[1]);
    show({"x", "y"}, 3);
    n = 1'bx;
    c = n ? "yes" : "no";
    $display("[%s] %0d", c, c.len());
    $display("%0d %0d", a.compare("%0s"), a.icompare("%0S+%0S"));
  end
endmodule
