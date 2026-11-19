// exit: 1
// flags: -g2005
// Verilog-2005 has no parameter ports without a default (IEEE 1364-2005 A.1.3).
module m #(parameter A) ();
endmodule

module test;
  m #(.A(1)) u();
endmodule
