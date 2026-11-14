// User-defined primitives (IEEE 1800 29): combinational tables with ? and b, x outputs for
// unlisted inputs; sequential level- and edge-sensitive tables with -, r, f, (01), *, an
// initial value
primitive mux2(y, s, a, b);
  output y; input s, a, b;
  table
    // s a b : y
       0 0 ? : 0;
       0 1 ? : 1;
       1 ? 0 : 0;
       1 ? 1 : 1;
       x 0 0 : 0;
       x 1 1 : 1;
  endtable
endprimitive
primitive latch(q, en, d);
  output q; reg q; input en, d;
  table
    // en d : q : q+
       1  0 : ? : 0;
       1  1 : ? : 1;
       0  ? : ? : -;
  endtable
endprimitive
primitive dff(q, c, d);
  output q; reg q; input c, d;
  initial q = 1'b0;
  table
    // c    d : q : q+
       r    0 : ? : 0;
       r    1 : ? : 1;
       (0x) ? : ? : -;
       f    ? : ? : -;
       ?    * : ? : -;
  endtable
endprimitive
module t;
  integer errors = 0;
  reg s, a, b, en, d, c;
  wire y, q, qd;
  mux2 m(y, s, a, b);
  latch l(q, en, d);
  dff f(qd, c, d);
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %b want %b", line, got[0], want[0]);
      errors = errors + 1;
    end
  endtask
  initial begin
    #1 check(`__LINE__, qd, 0);             // the initial value
    s = 0; a = 1; b = 0; en = 1; d = 1; c = 0;
    #1;
    check(`__LINE__, y, 1);
    check(`__LINE__, q, 1);
    s = 1; #1 check(`__LINE__, y, 0);
    s = 1'bx; #1 check(`__LINE__, y, 1'bx);  // a, b differ: not in the table
    a = 0; #1 check(`__LINE__, y, 0);
    en = 0; d = 0; #1 check(`__LINE__, q, 1);   // holds
    c = 1; #1 check(`__LINE__, qd, 0);          // rising edge takes d = 0
    d = 1; #1 check(`__LINE__, qd, 0);          // a data change alone holds
    c = 0; #1 check(`__LINE__, qd, 0);          // falling edge holds
    c = 1; #1 check(`__LINE__, qd, 1);
    if (errors == 0) $display("PASSED");
  end
endmodule
