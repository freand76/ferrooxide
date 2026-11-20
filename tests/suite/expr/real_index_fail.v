// exit: 1
// An index is not real (IEEE 1800-2017 11.5.1).
module test;
  reg [7:0] v;
  real r = 1.0;
  initial $display("%b", v[r]);
endmodule
