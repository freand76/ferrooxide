// exit: 1
// Port connections are all by order or all by name (IEEE 1800-2017 23.3.2).
module test;
  wire a, b;
  m u(.a(a), b);
endmodule
module m(input a, input b);
endmodule
