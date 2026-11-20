// exit: 1
// A net cannot be of a class type (IEEE 1800-2017 6.7.1).
module test;
  class C;
  endclass
  wire C x;
  initial $display("FAILED");
endmodule
