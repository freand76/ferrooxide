// $sscanf with the format in a variable (IEEE 1800-2017 21.3.4.3: the format is an
// expression), and a format with x bits: -1.
module m;
  reg [8*8:1] f;
  string sf;
  integer a, b, code;
  initial begin
    f = "%d,%h";
    code = $sscanf("12,ff", f, a, b);
    if (code != 2 || a != 12 || b != 255) $display("FAILED -- vector format %0d %0d %0d", code, a, b);
    sf = "x=%d";
    code = $sscanf("x=7", sf, a);
    if (code != 1 || a != 7) $display("FAILED -- string format %0d %0d", code, a);
    code = $sscanf("1", 'bx, a);
    if (code !== -1) $display("FAILED -- x format %0d", code);
    $display("PASSED");
  end
endmodule
