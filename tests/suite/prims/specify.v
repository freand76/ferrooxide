// expected
// flags: -gspecify
// specify blocks (IEEE 1800 30): specparams, parallel (=>) and full (*>) path delays with
// rise/fall values; timing checks are accepted and ignored, as by the reference
`timescale 1ns/1ns
module buf2(input a, input b, output y, output z);
  assign y = a;
  assign z = a & b;
  specify
    specparam T = 3;
    (a => y) = (T, 5);
    (a, b *> z) = 2;
    $setup(b, posedge a, 2);
    $hold(posedge a, b, 1);
    $width(posedge a, 4);
  endspecify
endmodule
module t;
  reg a = 0, b = 1; wire y, z;
  buf2 u(a, b, y, z);
  initial begin
    $monitor("%0t a=%b b=%b y=%b z=%b", $time, a, b, y, z);
    #10 a = 1;
    #10 a = 0;
    #10 b = 0; #1 a = 1;
    #10 $finish;
  end
endmodule
