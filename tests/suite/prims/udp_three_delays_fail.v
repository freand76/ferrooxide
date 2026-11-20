// exit: 1
// A UDP instance has at most two delays (rise, fall): its output is never z.
module test;
  reg a;
  wire y;
  inv #(1, 2, 3) u(y, a);
  initial $display("FAILED");
endmodule
primitive inv(output y, input a);
  table
    0 : 1;
    1 : 0;
  endtable
endprimitive
