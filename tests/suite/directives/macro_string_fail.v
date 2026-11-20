// exit: 1
// A macro definition or call with a string literal that does not end in the macro text (IEEE 1800-2017 22.5.1).
`define S "open
module test;
  initial $display(1);
endmodule
