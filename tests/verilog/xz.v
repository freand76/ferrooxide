module tb;
  reg [7:0] r;
  wire [7:0] w;
  wire [3:0] u;
  reg [3:0] q;
  initial begin
    $display("%b %h %d", r, r, r);
    $display("%b %h %d", w, w, w);
    q = 4'b1x0z;
    $display("%b %h %d", q, q, q);
    r = {q, 4'b0101};
    $display("%b %h %d", r, r, r);
    $display("%b", q & 4'b0011);
    $display("%b", q | 4'b1100);
    $display("%b", q ^ 4'b1111);
    $display("%b", ~q);
    $display("%b %b %b", &q, |q, ^q);
    $display("%b %b", q == 4'b1x0z, q === 4'b1x0z);
    $display("%b %b", q == 4'b0000, q != 4'b1101);
    $display("%b", q + 1);
    $display("%b", q ? 4'b1010 : 4'b1001);
    r = 8'b1x;
    $display("%b", r);
    r = 8'bx;
    $display("%b", r);
    r = 'hz;
    $display("%b", r);
    if (q[2]) $display("bad"); else $display("x is false");
  end
endmodule
