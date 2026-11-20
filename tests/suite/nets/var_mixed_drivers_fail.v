// exit: 1
// A variable is not written by both a continuous and a procedural assignment (IEEE 1800-2017 6.5).
module test;
  int v;
  assign v = 12;
  initial v = 1;
endmodule
