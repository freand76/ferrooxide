// Writes to parts of array elements at run-time offsets: mem[i][b +: w], mem[i][b]
module top;
  reg [15:0] mem [0:3];
  reg [3:0] q [1:2];
  integer i, b;
  initial begin
    for (i = 0; i < 4; i = i + 1) mem[i] = 0;
    q[1] = 0; q[2] = 0;
    i = 2; b = 4;
    mem[i][b +: 4] = 4'ha;
    mem[1][b*2 -: 3] = 3'b111;
    mem[3][b] = 1'b1;
    mem[0][14 +: 4] = 4'hf;          // partly out of range
    mem[i][20 +: 4] = 4'hf;          // out of range: nothing
    q[i-1][b-3 +: 2] <= 2'b11;
    q[2][b-4] <= 1'b1;
    #1 $display("%h %h %h %h %b %b", mem[0], mem[1], mem[2], mem[3], q[1], q[2]);
  end
endmodule
