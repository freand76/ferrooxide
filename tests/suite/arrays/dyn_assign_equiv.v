// Arrays with equivalent element types assign to each other (IEEE 1800-2017 6.22.2).
module test;
  typedef enum logic [7:0] { A, B } e_t;
  e_t e1[], e2[];
  byte b1[$];
  bit signed [7:0] b2[$];
  int i1[];
  bit signed [31:0] i2[];
  initial begin
    e2 = '{B, A};
    e1 = e2;
    b2 = '{-1, 2};
    b1 = b2;
    i2 = new[2];
    i2[1] = -5;
    i1 = i2;
    if (e1[0] == B && b1[0] == -1 && i1[1] == -5) $display("PASSED");
    else $display("FAILED");
  end
endmodule
