// Multi-dimensional arrays: element, bit and part access, constant and run-time indices, reals
module top;
  real rr [0:1][0:2];
  reg [7:0] m [0:2][1:4];
  reg [3:0] c [3:0][0:1][0:1];
  integer i, j, k;
  wire [7:0] w = m[1][2];
  initial begin
    for (i = 0; i < 3; i = i + 1)
      for (j = 1; j <= 4; j = j + 1)
        m[i][j] = i * 16 + j;
    for (i = 0; i < 4; i = i + 1)
      for (j = 0; j < 2; j = j + 1)
        for (k = 0; k < 2; k = k + 1)
          c[i][j][k] <= i + j * 2 + k * 4;
    rr[1][2] = -0.5; rr[0][1] = 2.25;
    #1;
    $display("%f %f %f", rr[1][2], rr[0][1], rr[0][0]);
    $display("%h %h %h %h", m[0][1], m[2][4], m[1][3], w);
    m[1][2][7:4] = 4'ha;
    m[2][1][0] = 1'b0;
    $display("%h %h %b", m[1][2], m[2][1], m[1][2][3:0]);
    $display("%h %h %h", c[3][1][1], c[0][0][1], c[2][1][0]);
    i = 2; j = 3;
    $display("%h %h", m[i][j], m[i-1][j+1]);
  end
endmodule
