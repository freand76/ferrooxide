// exit: 1
// A part-select bound is a constant expression (IEEE 1800-2017 11.5.1).
module test;
  reg [7:0] v;
  integer n = 3;
  initial $display("%b", v[n:0]);
endmodule
