// exit: 1
// flags: -gspecify
// A parallel path (`=>`) connects terminals of the same width (IEEE 1800-2017 30.4.5).
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
