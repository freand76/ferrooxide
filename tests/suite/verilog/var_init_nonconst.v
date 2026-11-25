// expected
// Variable initializers that are not constant expressions (IEEE 1800-2017 6.8): string
// methods, other variables, function calls; set before the processes start.
module top;
  string a = "Test";
  string b = "TEST";
  int c = a.compare(b);
  int n = a.len();
  integer k = 3;
  integer d = k * 2;
  function integer twice(input integer x); twice = 2 * x; endfunction
  integer e = twice(d);
  real r = d / 4.0;
  string u = a.toupper();
  initial $display("%0d %0d %0d %0d %f %s", c > 0, n, d, e, r, u);
  always @(d) $display("d changed to %0d", d);
endmodule
