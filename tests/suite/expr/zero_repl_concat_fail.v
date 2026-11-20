// exit: 1
// A concatenation needs at least one operand of positive width (IEEE 1800-2017 11.4.12.1).
module test;
  reg [7:0] r;
  initial r = {{0{1'b1}}};
endmodule
