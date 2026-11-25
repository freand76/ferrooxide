// expected
// A sequential UDP's `initial` output value is a change at time 0 that processes already
// waiting on the output see; the table is evaluated from the first input change on.
primitive latch(q, e, d);
  output q;
  input e, d;
  reg q;
  initial q = 1'b0;
  table
  // e d : q : q+
     1 1 : ? : 1;
     1 0 : ? : 0;
     0 ? : ? : -;
  endtable
endprimitive

module test;
  wire q;
  reg e, d;
  reg r;
  integer n = 0;
  latch l(q, e, d);
  always @(q) begin
    r = q;
    n = n + 1;
    $display("%0t q=%b", $time, q);
  end
  initial begin
    #1 $display("r=%b n=%0d", r, n);
    e = 1; d = 1;
    #1 e = 0;
    #1 d = 0;
    #1 e = 1;
    #1 $display("q=%b n=%0d", q, n);
  end
endmodule
