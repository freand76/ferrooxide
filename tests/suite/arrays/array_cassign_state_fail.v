// exit: 1
// An unpacked array assigned to another needs equivalent element types: logic and bit
// are not (IEEE 1800-2017 6.22.2, 7.6).
module test;
  wire [1:0] x[1:0];
  bit [1:0] y[1:0];
  assign x = y;
endmodule
