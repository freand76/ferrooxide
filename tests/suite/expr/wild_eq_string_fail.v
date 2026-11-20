// exit: 1
// The operands of a wildcard equality are integral (IEEE 1800-2017 11.4.6).
module test;
  string s = "a";
  reg [7:0] v = 8'h61;
  reg res;
  initial res = s !=? v;
endmodule
