// exit: 1
// Each port of a non-ANSI port list is declared with a direction.
module test;
  wire [3:0] a, y;
  m u(.a(a), .y(y));
endmodule
module m(a, b, y);
  input [3:0] a;
  output [3:0] y;
  assign y = a;
endmodule
