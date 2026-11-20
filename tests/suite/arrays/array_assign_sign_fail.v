// exit: 1
// An unpacked array assigned to another needs elements of the same signedness
// (IEEE 1800-2017 6.22.2, 7.6).
module test;
  logic [7:0] x[2];
  logic signed [7:0] y[2];
  initial x = y;
endmodule
