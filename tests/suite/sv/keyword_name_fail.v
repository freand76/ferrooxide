// exit: 1
// A keyword cannot be a name (IEEE 1800-2017 5.6.2): `bit` is a SystemVerilog keyword.
module test;
  reg bit;
  initial $display("FAILED");
endmodule
