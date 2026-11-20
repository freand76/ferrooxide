// exit: 1
// posedge / negedge need an integral value (IEEE 1800-2017 9.4.2).
module test;
  real a = 0.5;
  always @(posedge a) $display("FAILED");
endmodule
