// exit: 1
// A macro definition or call with a missing argument without a default (IEEE 1800-2017 22.5.1).
`define D(x, y) x + y
module test;
  initial $display(`D(1));
endmodule
