// exit: 1
// A real cannot be an operand of a concatenation (IEEE 1800-2017 11.3.1), also as a target.
module t;
  real a, b;
  reg in;
  assign {a, b} = in;
endmodule
