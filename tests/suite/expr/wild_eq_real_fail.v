// exit: 1
// The operands of a wildcard equality are integral (IEEE 1800-2017 11.4.6).
module test;
  real r = 1.0;
  reg [1:0] v = 2'b01;
  reg res;
  initial res = v ==? r;
endmodule
