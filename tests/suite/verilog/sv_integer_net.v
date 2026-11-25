// expected
// Nets of the 4-state integer types (IEEE 1800-2017 6.7.1): `wire integer` is a signed
// 32-bit net, `wire time` an unsigned 64-bit one; they resolve several drivers like vectors.
module test;
  reg [7:0] a = 8'd200;
  reg en = 1;
  wire integer i = a - 300;
  wire time t = a * 2;
  wire integer z;
  assign z = en ? 32'd5 : 32'bz;
  assign z = en ? 32'bz : -32'sd7;
  initial begin
    #1 $display("%0d %0d %0d %0d %0d", i, t, z, $bits(i), $bits(t));
    en = 0;
    #1 $display("%0d %b", z, i < 0);
  end
endmodule
