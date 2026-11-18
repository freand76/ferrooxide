// `++`/`--` inside expressions (IEEE 1800-2017 11.4.2): the prefix form gives the new value,
// the postfix form the old one; `&&`, `||` and `?:` evaluate only what decides the result.
module test;
  integer i, j, fails = 0;
  reg [1:0] k;
  reg [3:0] x;
  real r, s;
  int a[4];

  class Counter;
    int n;
    function int next();
      return n++;
    endfunction
  endclass

  task check(input integer got, input integer want, input integer line);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d, want %0d", line, got, want);
      fails++;
    end
  endtask

  Counter c = new;

  initial begin
    i = 5;
    j = i++;      check(j, 5, `__LINE__); check(i, 6, `__LINE__);
    j = ++i;      check(j, 7, `__LINE__); check(i, 7, `__LINE__);
    j = i-- + 10; check(j, 17, `__LINE__); check(i, 6, `__LINE__);
    j = --i * 2;  check(j, 10, `__LINE__); check(i, 5, `__LINE__);
    k = 3;
    j = k++;      check(j, 3, `__LINE__); check(k, 0, `__LINE__);   // wraps at the width
    x = 4'bxxxx;
    i = 0;
    x[i++] = 1'b0;
    check(x[0], 0, `__LINE__); check(i, 1, `__LINE__);
    a[0] = 1; a[1] = 2;
    i = 0;
    j = a[i++] + a[i++];
    check(j, 3, `__LINE__); check(i, 2, `__LINE__);
    r = 1.5;
    s = r++;
    if (s != 1.5 || r != 2.5) begin $display("FAILED real postfix %f %f", s, r); fails++; end
    s = --r;
    if (s != 1.5 || r != 1.5) begin $display("FAILED real prefix %f %f", s, r); fails++; end
    // short-circuit
    i = 0;
    j = 0 && i++;  check(i, 0, `__LINE__); check(j, 0, `__LINE__);
    j = 1 || i++;  check(i, 0, `__LINE__); check(j, 1, `__LINE__);
    j = 1 && i++;  check(i, 1, `__LINE__); check(j, 0, `__LINE__);
    j = 0 || ++i;  check(i, 2, `__LINE__); check(j, 1, `__LINE__);
    j = 1'bx && i++; check(i, 3, `__LINE__); check(j, 1'bx, `__LINE__);
    j = i > 0 ? i++ : i--; check(i, 4, `__LINE__); check(j, 3, `__LINE__);
    // a class property in a method
    j = c.next(); j = c.next();
    check(j, 1, `__LINE__); check(c.n, 2, `__LINE__);
    if (fails == 0) $display("PASSED");
  end
endmodule
