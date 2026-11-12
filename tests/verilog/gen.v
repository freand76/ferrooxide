module inv #(parameter N=1) (input [N-1:0] a, output [N-1:0] y);
  assign y = ~a;
endmodule
module tb;
  reg [7:0] a;
  wire [7:0] y, y2;
  genvar i;
  generate
    for (i = 0; i < 8; i = i + 1) begin : bits
      inv u(.a(a[i]), .y(y[i]));
    end
  endgenerate
  inv #(8) whole(a, y2);
  wire [3:0] g;
  and g0(g[0], a[0], a[1]);
  or  g1(g[1], a[0], a[1]);
  xor g2(g[2], a[0], a[1], a[2]);
  not g3(g[3], a[0]);
  initial begin
    a = 8'h5a;
    #1 $display("%h %h %b", y, y2, g);
    a = 8'h0f;
    #1 $display("%h %h %b", y, y2, g);
  end
endmodule
