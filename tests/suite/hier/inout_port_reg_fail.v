// exit: 1
// An inout port of a module is a net (IEEE 1800-2017 23.2.2.3): a non-ANSI port cannot be
// declared again as a variable.
module c(d);
  inout [3:0] d;
  reg [3:0] d;
endmodule
module t;
  wire [3:0] w;
  c u(w);
endmodule
