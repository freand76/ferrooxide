// exit: 1
// The selects in the target of a force are constant (IEEE 1800-2017 10.6.2).
module test;
  wire [7:0] w;
  integer n = 1;
  initial force w[n] = 1'b1;
endmodule
