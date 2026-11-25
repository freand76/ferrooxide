// expected
// interface port arrays connected to arrays of other ranges (ICARUS_NOTES 10)
interface I; logic [7:0] v; endinterface
module m(I p[1:0], output [7:0] a, output [7:0] b);
  assign a = p[1].v; assign b = p[0].v;
endmodule
module m2(I p[0:1], output [7:0] a, output [7:0] b);
  assign a = p[0].v; assign b = p[1].v;
endmodule
module t;
  I x[3:2](); I y[2:3]();
  assign x[3].v = 3; assign x[2].v = 2;
  assign y[3].v = 13; assign y[2].v = 12;
  wire [7:0] a, b, c, d, e, f;
  m u(x, a, b);
  m u2(y, c, d);
  m2 u3(x, e, f);
  initial #1 $display("%0d %0d | %0d %0d | %0d %0d", a, b, c, d, e, f);
endmodule
