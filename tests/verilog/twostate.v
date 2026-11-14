// 2-state types (bit, byte, shortint, int, longint): start at 0, x/z stored become 0
module t;
  typedef struct packed { bit [3:0] a; bit b; } bs_t;
  typedef enum bit [1:0] {A, B, C} be_t;
  bit b1;
  bit [7:0] bv;
  byte by;
  shortint sh;
  int i;
  longint li;
  bit [3:0] mem [0:3];
  bs_t bs;
  be_t be;
  logic [7:0] lg;
  bit [7:0] cont;
  int k, ri;
  real r;
  assign cont = lg;
  initial begin
    $display("init %b %b %0d %0d %0d %0d %b %b %0d", b1, bv, by, sh, i, li, mem[2], bs, be);
    lg = 8'b1x0z_10x1;
    b1 = 1'bx;
    bv = lg;
    by = 8'hxz;
    i = 32'bx;
    i[3:0] = 4'b1x1z;
    sh = -1;
    sh[15:8] = 8'hzz;
    mem[1] = 4'bx1x1;
    bs = 5'bx1z01;
    be = be_t'(lg[2:1]);
    #1 $display("x/z %b %b %b %b %b %b %b %b %b", b1, bv, by, i[7:0], sh, mem[1], bs, be, cont);
    bv <= 8'bxxxx1111;
    #1 $display("nba %b", bv);
    k = 0;
    for (int j = 0; j < 4; j++) k += j;
    i = 5 + 1'bx;
    $display("sum %0d i %0d b1==0 %b isunknown %b", k, i, b1 == 1'b0, $isunknown(i));
    // the reference keeps x when an infinite real is converted to int (ICARUS_NOTES.md 7)
    r = 1.0 / 0.0;
    ri = r;
    r = 2.5;
    i = r;
    $display("inf -> int %0d, 2.5 -> int %0d", ri, i);
  end
endmodule
