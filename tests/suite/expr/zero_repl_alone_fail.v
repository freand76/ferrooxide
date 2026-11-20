// exit: 1
// A replication with a zero count is allowed only as an operand of a concatenation
// (IEEE 1800-2017 11.4.12.1).
module test;
  reg [7:0] r;
  initial r = {0{1'b1}};
endmodule
