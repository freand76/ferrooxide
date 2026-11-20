// exit: 1
// The selects in the target of a continuous assignment are constant (IEEE 1800-2017 10.3.1).
module test;
  wire [7:0] w;
  reg [2:0] n;
  assign w[n] = 1'b1;
endmodule
