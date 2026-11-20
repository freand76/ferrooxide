// Selects that are legal: a bit of an array element, a part of an element of a packed
// two-dimensional vector, a bit of an element of a two-dimensional unpacked array.
module test;
  reg [15:0] mem [3:0];
  reg [3:0][7:0] p;
  reg [7:0] m2 [1:0][1:0];
  initial begin
    mem[1] = 16'h8001;
    p = 32'h12345678;
    m2[1][0] = 8'h80;
    if (mem[1][0] === 1'b1 && mem[1][15] === 1'b1 && p[1][3:0] === 4'h6 && p[2][7] === 1'b0 && m2[1][0][7] === 1'b1)
      $display("PASSED");
    else
      $display("FAILED");
  end
endmodule
