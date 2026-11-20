// exit: 1
// A port gets at most one port declaration.
module m(x);
  input x;
  input x;
endmodule
module test;
  m u(1'b0);
endmodule
