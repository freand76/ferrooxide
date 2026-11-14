// expected
// $display formats (IEEE 1800 21.2.1): radixes with x/z digits, field widths (%0d, %5d,
// automatic widths), %s, %c, %e/%f/%g, %m, %%, default decimal arguments, $write
module t;
  reg [11:0] v;
  reg signed [7:0] s;
  real r;
  initial begin
    v = 12'h5a3;
    $display("[%b] [%o] [%h] [%d] [%0d] [%6d] [%0h]", v, v, v, v, v, v, v);
    v = 12'b0101_xxxx_zz10;
    $display("[%b] [%h] [%o] [%d]", v, v, v, v);
    v = 12'bxxxx_xxxx_xxxx;
    $display("[%h] [%d] [%0d]", v, v, v);
    v = 12'bzzzz_zzzz_zzzz;
    $display("[%h] [%d]", v, v);
    v = 12'b0000_0000_10x1;
    $display("[%d]", v);
    s = -5;
    $display("[%d] [%0d] [%h] [%b]", s, s, s, s);
    $display("[%s] [%5s] [%c]", "hi", "ab", 8'h41);
    r = 3.14159;
    $display("[%f] [%.2f] [%e] [%g] [%0.1f] [%8.3f]", r, r, r, r, r, r);
    r = 1.0e10;
    $display("[%g] [%e]", r, r);
    $display("[%m] [100%%] [%t]", $time);
    $display(v, s, "x");
    $write("a");
    $write("b\n");
    $display("tab\there \\ \"q\"");
    $display("%0b %0o", 12'h001, 12'o0017);
    $display("[%x] [%X]", 8'hab, 8'hab);
  end
endmodule
