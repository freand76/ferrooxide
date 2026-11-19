// exit: 1
// Overriding a parameter the module does not have is an error.
module sub;
  parameter A = 1;
endmodule
module test;
  sub #(.C(7)) u();
endmodule
