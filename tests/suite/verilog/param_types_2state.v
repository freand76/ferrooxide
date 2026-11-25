// expected
// Parameter values converted to the parameter's type: width, sign, and no x or z bits in
// a 2-state type (bit, int, byte, shortint); also for overrides.
module sub #(parameter bit [3:0] P = 0, parameter int I = 0);
  initial $display("sub: %b %b", P, I);
endmodule
module t;
  parameter bit [3:0] P = 4'bx01z;
  localparam bit Q = 1'bx;
  localparam int I = 'x;
  localparam byte B = 8'hzz;
  localparam bit signed [7:0] S = 8'b1x1x_0000;
  localparam bit Q1 = 4'b1010;
  localparam logic Q2 = 4'b1011;
  localparam int unsigned U = -1;
  localparam integer J = 4'bx;
  localparam real R = 3;
  localparam shortint SH = 17'h1ffff;
  localparam logic [3:0] L = 4'bx01z;
  sub #(.P(4'b1z0x), .I(32'hx)) s();
  initial begin
    $display("%b %b %b %b %b", P, Q, I, B, S);
    $display("%b %b %0d %b %f %0d %b", Q1, Q2, U, J, R, SH, L);
  end
endmodule
