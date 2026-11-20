// exit: 1
// An unsigned enum cannot have a negative value (IEEE 1800-2017 6.19).
module test;
  enum bit [1:0] {a = -1, b} v;
  initial $display("FAILED");
endmodule
