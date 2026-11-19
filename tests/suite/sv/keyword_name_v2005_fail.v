// exit: 1
// A keyword cannot be a name (IEEE 1800-2017 5.6.2); `begin_keywords "1364-2005"
// (22.14) frees the names SystemVerilog added, not Verilog-2005's own (`small`).
`begin_keywords "1364-2005"
module test;
  reg small;
  initial $display("FAILED");
endmodule
`end_keywords
