// Reals as conditions: true when not 0.0 (also Inf and NaN)
module top;
  real z, i, n, h;
  reg r;
  initial begin
    z = 0.0; h = 0.5; i = 1.0 / z; n = i - i;
    $display("%b %b %b %b", !z, !h, !i, !n);
    $display("%b %b %b", h && i, z || n, z && i);
    $display("%0d %0d", h ? 1 : 2, z ? 1 : 2);
    if (h) $display("if h");
    if (z) $display("if z"); else $display("else z");
    while (h) h = h - 0.25;
    $display("%f", h);
  end
endmodule
