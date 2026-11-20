// exit: 1
// A real is not an operand of a concatenation (IEEE 1800-2017 11.4.12).
module test;
  real r = 1.0;
  initial $display("%b", {r, 1'b0});
endmodule
