// exit: 1
// With a parameter port list, a module's own parameters are local (IEEE 1800-2017 6.20.1):
// overriding one is an error.
module sub #(parameter A = 1) ();
  parameter B = 2;
endmodule
module test;
  sub #(.A(5), .B(7)) u();
endmodule
