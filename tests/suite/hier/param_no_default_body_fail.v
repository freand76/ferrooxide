// exit: 1
// Only a parameter port may leave out its default (IEEE 1800-2017 6.20.1); a parameter
// declared in the module body needs a value even when every instance overrides it.
module m;
  parameter A;
endmodule

module test;
  m #(.A(10)) u();
endmodule
