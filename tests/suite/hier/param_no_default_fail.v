// exit: 1
// A parameter port without a default must be overridden by every instance (IEEE 1800-2017
// 6.20.1).
module m #(parameter W) (output [W-1:0] o);
endmodule

module test;
  wire a;
  m u1(a);
endmodule
