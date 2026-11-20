// exit: 1
// An enum value must fit the enum's base type (IEEE 1800-2017 6.19): here the implicit
// value after 3 does not fit 2 bits.
module test;
  enum bit [1:0] {a = 3, b} v;
  initial $display("FAILED");
endmodule
