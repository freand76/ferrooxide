// expected
// A string stored into a vector whose width is not a multiple of 8 keeps the low bits of
// the character that only partly fits.
// args: +string=0123456789
module top;
  reg [64:0] str;
  reg [11:0] s12;
  reg [69:0] s70;
  initial begin
    if ($value$plusargs("string=%s", str)) $display("%h", str);
    $sformat(s12, "%s", "abc");
    $display("%h", s12);
    $sformat(s70, "%s", "0123456789");
    $display("%h", s70);
  end
endmodule
