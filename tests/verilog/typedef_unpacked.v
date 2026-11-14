// Typedefs with unpacked dimensions: the type's dimensions come before the declaration's own
// (as in the reference), nested array typedefs, $bits of an array type.
module t;
  typedef logic [7:0] T1[3];
  typedef T1 T2[2];
  T1 x[5];
  T2 y;
  integer i, j;
  initial begin
    $display("%0d %0d %0d", $unpacked_dimensions(x), $size(x, 1), $size(x, 2));
    $display("%0d %0d %0d", $unpacked_dimensions(y), $size(y, 1), $size(y, 2));
    $display("%0d %0d", $bits(T1), $bits(T2));
    for (i = 0; i < 3; i = i + 1)
      for (j = 0; j < 5; j = j + 1)
        x[i][j] = i * 16 + j;
    $display("%h %h %h", x[0][0], x[2][4], x[1][3]);
    y[2][1] = 8'h5a;
    $display("%h %h", y[2][1], y[0][0]);
  end
endmodule
