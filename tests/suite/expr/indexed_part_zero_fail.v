// exit: 1
// An indexed part-select has a positive width (IEEE 1800-2017 11.5.1).
module test;
  parameter integer c = 0;
  reg [7:0] a, b;
  initial b[4+:c] = a[1+:c];
endmodule
