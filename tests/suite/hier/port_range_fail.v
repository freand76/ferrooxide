// exit: 1
// A port declared without a range must not get one from its net declaration (IEEE
// 1364-2005 12.3.3: the ranges shall be identical).
module m(a);
  input a;
  wire [7:0] a;
endmodule
module test;
  m u(8'd1);
endmodule
