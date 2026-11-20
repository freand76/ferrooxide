// exit: 1
// A sized value of an enum constant has the base type's width (IEEE 1800-2017 6.19).
module test;
  enum logic [3:0] { A, B = 3'b101 } e;
endmodule
