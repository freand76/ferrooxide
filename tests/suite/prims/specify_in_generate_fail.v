// exit: 1
// A generate region may not hold a specify block (IEEE 1800-2017 27.2).
module test;
  generate
    specify
    endspecify
  endgenerate
endmodule
