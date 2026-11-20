// exit: 1
// An inout port has no default value (IEEE 1800-2017 23.2.2.4).
module m(inout [31:0] x, y = 1);
endmodule
module test;
  wire [31:0] x, y;
  m u(.x(x), .y(y));
endmodule
