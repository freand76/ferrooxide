// exit: 1
// A generate region cannot be inside another one (IEEE 1800-2017 27.3).
module test;
  wire y;
  reg in;
  generate
    generate
      assign y = in;
    endgenerate
  endgenerate
endmodule
