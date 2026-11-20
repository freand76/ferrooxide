// exit: 1
// $signed takes an integral argument (IEEE 1800-2017 11.7).
module test;
  reg [7:0] r;
  initial r = $signed(1.0);
endmodule
