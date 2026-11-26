// exit: 1
// A parallel path (`=>`) connects terminals of the same width (IEEE 1800-2017 30.4.5),
// also when path delays are not applied (no -gspecify).
module test;
  reg in;
  wire [7:0] out;
  m u(out, in);
endmodule
module m(output [7:0] out, input in);
  assign out = {8{in}};
  specify
    (in => out) = 2;
  endspecify
endmodule
