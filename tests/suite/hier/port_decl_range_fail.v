// exit: 1
// The net or variable declaration of a port has the range of its port declaration.
module m(x);
  output [3:0] x;
  reg [7:0] x;
endmodule
module test;
  m u();
endmodule
