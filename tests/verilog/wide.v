// Wide (>64-bit) values through combinational and clocked processes (JIT wide ops).
module tb;
  reg clk = 0;
  reg [127:0] a, b;
  reg [199:0] c;
  reg signed [95:0] s;
  reg [1:0] sel;
  wire [127:0] sum = a + b;
  wire [127:0] dif = a - b;
  wire [127:0] band_ = a & b, bor_ = a | b, bxor_ = a ^ b, bnot_ = ~a;
  wire eq = a == b, ne = a != b, ceq = a === b, lt = a < b, le = a <= b, gt = a > b;
  wire [127:0] mx = sel[0] ? a : b;
  wire [199:0] cat = {a[100:37], b, c[7:0]};
  wire [70:0] sl = c[180:110];
  wire [63:0] lo = c[99:36];
  wire [159:0] sx = s;
  wire [159:0] zx = a;
  wire [40:0] tr = a;
  wire nz = |c;
  reg [127:0] q, mem [0:3];
  reg [127:0] r2;
  always @(posedge clk) begin
    q <= sum ^ {64'h1, a[63:0]};
    mem[1] <= dif;
    mem[2] <= mem[1];
  end
  always @* r2 = (a > b) ? a - b : b - a;
  task show;
    begin
      #1;
      $display("sum=%h dif=%h", sum, dif);
      $display("and=%h or=%h xor=%h not=%h", band_, bor_, bxor_, bnot_);
      $display("eq=%b ne=%b ceq=%b lt=%b le=%b gt=%b mx=%h", eq, ne, ceq, lt, le, gt, mx);
      $display("cat=%h sl=%h lo=%h", cat, sl, lo);
      $display("sx=%h zx=%h tr=%h nz=%b r2=%h", sx, zx, tr, nz, r2);
    end
  endtask
  initial begin
    a = 128'hffff_ffff_ffff_ffff_0000_0000_0000_0001; b = 128'h1; c = 0; s = -96'sd5; sel = 1;
    show;
    a = 128'h0123_4567_89ab_cdef_fedc_ba98_7654_3210; b = 128'hffff_0000_ffff_0000_ffff_0000_ffff_0000;
    c = {8'h5a, 64'hdead_beef_cafe_babe, 64'h0123_4567_89ab_cdef, 64'h1111_2222_3333_4444}; s = 96'sh7fff_0000_0000_0000_0000_0001; sel = 0;
    show;
    a[70] = 1'bx; b[3] = 1'bz; sel = 2'bx1;
    show;
    sel = 2'b1x;
    show;
    a = 128'h5; b = 128'h5;
    show;
    clk = 1; #1 clk = 0; #1 clk = 1; #1 clk = 0;
    #1 $display("q=%h m1=%h m2=%h", q, mem[1], mem[2]);
    a = {128{1'bx}};
    clk = 1; #1 clk = 0;
    #1 $display("q=%h m1=%h m2=%h", q, mem[1], mem[2]);
  end
endmodule
