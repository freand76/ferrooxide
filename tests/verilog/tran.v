// Bidirectional connections: inout ports on parts of nets, tran / tranif
module drv (inout [1:0] p, input en, input [1:0] v);
  assign p = en ? v : 2'bzz;
endmodule

module bit1 (inout b, input en);
  assign b = en ? 1'b1 : 1'bz;
endmodule

module top;
  wire [3:0] bus;
  reg en0, en1, en2;
  reg [1:0] v0, v1;
  reg [3:0] ext;
  assign bus = ext;
  drv d0 (bus[1:0], en0, v0);
  drv d1 ({bus[3], bus[2]}, en1, v1);
  wire q;
  bit1 b (q, en2);
  wire a, c, e;
  reg da, dc, ctl;
  assign a = da;
  assign c = dc;
  tran t1 (a, e);
  wire f;
  tranif1 t2 (e, f, ctl);
  rtranif0 t3 (f, c, ctl);
  initial begin
    ext = 4'bzzzz; en0 = 0; en1 = 0; en2 = 0; v0 = 2'b01; v1 = 2'b10;
    da = 1'bz; dc = 1'bz; ctl = 0;
    #1 $display("%b %b | %b %b %b %b", bus, q, a, e, f, c);
    en0 = 1; en2 = 1; da = 1;
    #1 $display("%b %b | %b %b %b %b", bus, q, a, e, f, c);
    en1 = 1; ext = 4'b0zzz; ctl = 1;
    #1 $display("%b %b | %b %b %b %b", bus, q, a, e, f, c);
    ext = 4'bzzzz; da = 1'bz; dc = 0; ctl = 0;
    #1 $display("%b %b | %b %b %b %b", bus, q, a, e, f, c);
    ctl = 1;
    #1 $display("%b %b | %b %b %b %b", bus, q, a, e, f, c);
  end
endmodule
