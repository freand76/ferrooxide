// exit: 1
// A bit select of a one-dimensional vector has nothing left to select from.
module test;
  reg [15:0] v;
  reg [3:0] r;
  initial r = v[0][3:0];
endmodule
