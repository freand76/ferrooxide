// exit: 1
// a net's data type is 4-state (IEEE 1800-2017 6.7.1): `wire int` is an error
module top;
  wire int w = 1;
endmodule
