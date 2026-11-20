// exit: 1
// A defparam must name a parameter of the target instance (IEEE 1800-2017 23.10.1).
module a #(parameter A = 1);
endmodule
module test;
  a i_a();
  defparam i_a.Z = 10;
endmodule
