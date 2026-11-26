// expected
// an unpacked array assigned from a conditional of arrays (IEEE 1800-2017 11.4.11): the
// selected array, or with an unknown condition the elements combined bit by bit
module top;
  localparam d = 2, t = 0;
  localparam int m [d:0] = t ? '{5, 4, 0} : '{7, 3, 0};
  logic [3:0] a [2], b [2], v [2];
  logic c;
  initial begin
    $display("%0d %0d %0d", m[2], m[1], m[0]);
    a = '{4'b1010, 4'b0011};
    b = '{4'b1001, 4'b0011};
    c = 1;
    v = c ? a : b;
    $display("%b %b", v[0], v[1]);
    c = 0;
    v = c ? a : b;
    $display("%b %b", v[0], v[1]);
    c = 1'bx;
    v = c ? a : b;
    $display("%b %b", v[0], v[1]);
    v = c ? '{4'b1111, 4'b0000} : a;
    $display("%b %b", v[0], v[1]);
  end
endmodule
