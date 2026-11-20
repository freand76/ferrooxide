// exit: 1
// A variable takes at most one continuous assignment (IEEE 1800-2017 6.5).
module test;
  int v;
  assign v = 12;
  assign v = 13;
endmodule
