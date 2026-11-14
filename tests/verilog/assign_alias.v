// A plain `assign z = a;` makes z another name for a (it changes inside the writing process);
// ports and aliases keep their own declared range and sign.
module sub(input wire i, output wire o);
  assign o = i;
endmodule
module view(input wire [0:3] p, input wire signed [3:0] s, output [4:1] o);
  initial #5 $display("view %b %b %0d %b", p[0], p[1:2], s + 0, o[4]);
  assign o = 4'b1000;
endmodule
module t;
  reg a, b; reg [1:0] r; reg m [0:1]; reg signed sa; reg [3:0] x, y;
  wire z, w, y1, q, n, sg, z1, z5, z4, zf;
  wire [1:0] v; wire [3:0] wide; wire [3:0] vo;
  wire [0:1] rv; assign rv = r;       // own range
  assign z = a;
  assign w = a & b;                   // logic: updated in the settle
  assign v = r;
  assign y1 = z;                      // chain
  sub u(.i(a), .o(q));
  assign n = ~a;
  assign sg = sa;
  assign wide = r;                    // widened: not an alias
  wire z2 = a;
  assign z1 = {a};
  assign z5 = m[0];
  assign #0 z4 = a;
  assign zf = a;                      // forced below: not an alias
  logic lz; assign lz = a;            // a variable with a continuous assignment
  view vw(.p(x), .s(y), .o(vo));
  initial begin
    a = 0; b = 1; r = 2; sa = 1; m[0] = 0; x = 4'b1000; y = 4'b1111;
    #1;
    a = 1; $display("z=%b w=%b y1=%b q=%b n=%b z1=%b z2=%b z4=%b", z, w, y1, q, n, z1, z2, z4);
    r = 1; $display("v=%b wide=%b rv[0]=%b", v, wide, rv[0]);
    sa = 0; m[0] = 1; $display("sg=%b z5=%b lz=%b", sg, z5, lz);
    #1 $display("z=%b w=%b y1=%b q=%b n=%b v=%b", z, w, y1, q, n, v);
    force zf = 0; #1 $display("zf=%b a=%b", zf, a);
    a = 0; #0 a = 1; #0 $display("zf=%b a=%b", zf, a);
    release zf; #0 $display("zf=%b a=%b", zf, a);
    #10 $finish;
  end
  initial $monitor("%0t mon z=%b", $time, z);
  always @(z) $display("%0t @z %b", $time, z);
  always @(zf) $display("%0t @zf %b", $time, zf);
endmodule
