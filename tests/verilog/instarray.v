// Arrays of gate and module instances
module inv #(parameter W = 2) (input [W-1:0] a, output [W-1:0] y, input en);
  assign y = en ? ~a : {W{1'bz}};
endmodule

module addone (input [3:0] a, output [3:0] y);
  assign y = a + 1;
endmodule

module top;
  reg [7:0] v;
  reg en;
  wire [7:0] b, n, x;
  wire [7:0] p;
  wire [0:3] q;
  wire [15:0] s;
  buf bb[7:0] (b, v);
  not nn[1:8] (n, v);
  and aa[3:0] (x[3:0], v[3:0], en);
  bufif1 bi[0:3] (q, v[7:4], en);
  inv #(.W(2)) iv[3:0] (.a(v), .y(p), .en(en));
  addone ad[3:0] ({v, v}, s);
  initial begin
    en = 1;
    v = 8'b1100_1010;
    #1 $display("%b %b %b %b %b %h", b, n, x[3:0], q, p, s);
    en = 0;
    v = 8'b0011_x10z;
    #1 $display("%b %b %b %b %b %h", b, n, x[3:0], q, p, s);
    $display("%b", iv[2].y);
  end
endmodule
