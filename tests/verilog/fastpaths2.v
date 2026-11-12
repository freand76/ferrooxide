// Store fast paths: part selects of wide signals (crossing words), part selects of array
// elements with variable (x, out of range) indices, non-blocking element stores seen through
// element-level sensitivity.
module tb;
  reg clk = 0;
  reg [199:0] wide;
  reg [15:0] mem [0:7];
  reg [3:0] ia;
  reg [15:0] w5;
  integer k;
  always @(mem[5]) w5 = mem[5];
  always @(posedge clk) begin
    mem[ia] <= mem[ia] + 16'h0101;
    mem[ia + 1][11:4] <= 8'hee;
  end
  initial begin
    wide = 0;
    for (k = 0; k < 8; k = k + 1) mem[k] = 16'h1000 * k;
    wide[70:60] = 11'h7ff;
    wide[199:190] = 10'h155;
    wide[8:1] = 8'h81;
    $display("%h", wide);
    ia = 3; mem[ia][7:0] = 8'hab; mem[ia][15:12] = 4'h9;
    ia = 4'bx; mem[ia][3:0] = 4'hf;
    ia = 12; mem[ia][3:0] = 4'hf;
    for (k = 0; k < 8; k = k + 1) $write("%h ", mem[k]);
    $display("");
    ia = 5;
    #1 clk = 1; #1 clk = 0;
    $display("w5=%h mem5=%h mem6=%h", w5, mem[5], mem[6]);
    ia = 4;
    #1 clk = 1; #1 clk = 0;
    $display("w5=%h mem5=%h mem4=%h", w5, mem[5], mem[4]);
    ia = 4'bx;
    #1 clk = 1; #1 clk = 0;
    for (k = 0; k < 8; k = k + 1) $write("%h ", mem[k]);
    $display("w5=%h", w5);
  end
endmodule
