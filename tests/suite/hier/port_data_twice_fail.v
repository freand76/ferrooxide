// exit: 1
// A port gets at most one net or variable declaration besides its port declaration.
module m(x);
  output x;
  reg x;
  reg x;
endmodule
module test;
  m u();
endmodule
