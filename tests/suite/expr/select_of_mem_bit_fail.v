// exit: 1
// mem[i][j] is one bit of an element; a further select has too many indices.
module test;
  reg [15:0] mem [3:0];
  reg [3:0] r;
  initial r = mem[0][0][3:0];
endmodule
