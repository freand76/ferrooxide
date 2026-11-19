// exit: 1
// Arrays assigned to each other need the same shape, not only the same number of
// elements (IEEE 1800-2017 7.6).
module test;
  logic [1:0] x[1:0][1:0];
  logic [1:0] y[3:0];
  initial x = y;
endmodule
