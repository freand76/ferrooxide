// exit: 1
// A macro definition or call with a directive name as a macro name (IEEE 1800-2017 22.5.1).
`define define 1
module test;
  initial $display(1);
endmodule
