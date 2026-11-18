// exit: 1
// An array port connected to an array of another index range (not supported yet).
module sub(input wire [7:0] d[0:3]);
endmodule
module test;
  logic [7:0] a[1:4];
  sub u(.d(a));
endmodule
