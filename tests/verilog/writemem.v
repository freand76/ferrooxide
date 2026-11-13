// $writememh/$writememb, read back with $readmemh/$readmemb
module top;
  reg [7:0] m [2:5];
  reg [7:0] r [0:3];
  reg [3:0] b [0:1];
  integer i;
  initial begin
    for (i = 2; i <= 5; i = i + 1) m[i] = i * 17;
    m[3] = 8'bx01z_0000;
    $writememh("wm_a.hex", m);
    $writememb("wm_b.bin", m, 3, 4);
    $writememh("wm_c.hex", m, 5, 3);
    $readmemh("wm_a.hex", r);
    $display("%h %h %h %h", r[0], r[1], r[2], r[3]);
    $readmemb("wm_b.bin", r);
    $display("%b %b", r[0], r[1]);
    $readmemh("wm_c.hex", r);
    $display("%h %h %h", r[0], r[1], r[2]);
  end
endmodule
