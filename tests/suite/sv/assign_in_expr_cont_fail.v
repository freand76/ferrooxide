// exit: 1
// an assignment in an expression only in procedural code (IEEE 1800-2017 11.3.6)
module top;
  int a, b;
  wire [31:0] w;
  assign w = (a = b);
endmodule
