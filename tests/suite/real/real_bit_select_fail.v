// exit: 1
// A real value has no bits to select (IEEE 1800-2017 6.12).
module test;
  real a = 0.5;
  wire b;
  assign b = a[2];
endmodule
