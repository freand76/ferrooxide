// exit: 1
// A macro definition or call with more arguments than parameters (IEEE 1800-2017 22.5.1).
`define D(x, y) x + y
module test;
  initial $display(`D(1, 2, 3));
endmodule
