// Legal enum values: an x value in a 4-state enum, sized values of the base width,
// unsized values, the same values in two instances of a module (IEEE 1800-2017 6.19).
module m;
  enum logic [3:0] { A = 4'd1, B, X = 4'bxxxx, C = 7 } e;
endmodule
module test;
  m m1(), m2();
  initial begin
    if (m1.B == 2 && m2.C == 7 && $isunknown(m1.X)) $display("PASSED");
    else $display("FAILED");
  end
endmodule
