// exit: 1
// A net cannot be a dynamic array (IEEE 1800-2017 6.7.1).
module test;
  wire x[];
  initial $display("FAILED");
endmodule
