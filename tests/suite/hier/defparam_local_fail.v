// exit: 1
// A defparam cannot change a local parameter; with a parameter port list, the body's
// parameters are local (IEEE 1800-2017 6.20.1, 23.10.1).
module a #(parameter A = 1);
  parameter B = 2;
endmodule
module test;
  a i_a();
  defparam i_a.B = 20;
endmodule
