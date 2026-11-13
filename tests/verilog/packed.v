// Packed multi-dimensional vectors: element, bit and slice selects, both directions
module top;
  reg [3:0][7:0] a;
  reg [0:3][7:0] d;
  reg [1:4][3:0][1:0] c;
  integer i;
  initial begin
    a = 32'h44332211; d = 32'h44332211; c = 32'h76543210;
    $display("%h %h %h %h", a[0], a[3], d[0], d[3]);
    $display("%b %b %h %h", a[1][0], d[1][7], a[2:1], d[1:2]);
    $display("%h %h %h %h", a[0+:2], a[3-:2], d[1+:2], d[2-:2]);
    $display("%h %h %b", c[1], c[4][3], c[2][1][0]);
    for (i = 0; i < 4; i = i + 1) a[i] = i + 8'ha0;
    a[2][7:4] = 4'h5;
    d[1+:2] = 16'hbeef;
    c[3][0] = 2'b11;
    $display("%h %h %h", a, d, c);
  end
endmodule
