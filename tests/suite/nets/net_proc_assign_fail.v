// exit: 1
// A net takes no procedural assignment (IEEE 1800-2017 10.4, table 10-1).
module test;
  wire w;
  initial w = 1;
endmodule
