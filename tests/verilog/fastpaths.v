// Fast paths with generic fallbacks: memory reads/writes with variable indices (in range, out of
// range, x, negative), variable bit selects past the end, constant part-select stores, logical
// not, signed compares, inside clocked and combinational processes.
module tb;
  reg clk = 0;
  reg [15:0] mem [0:7];
  reg [5:0] ia;
  integer is;
  reg [15:0] rd1, rd2, rd3;
  reg [31:0] word;
  reg [4:0] bi;
  reg b1, b2, nb;
  reg [3:0] lt;
  reg signed [7:0] sa, sb;
  integer k;
  always @* begin
    rd1 = mem[ia];
    rd2 = mem[is];
    b1 = word[bi];
    b2 = word[bi + 5'd30];
    nb = !word[7:0];
    lt = {sa < sb, sa <= sb, sa > sb, sa >= sb};
  end
  always @(posedge clk) begin
    mem[ia] = word[15:0];
    word[23:16] = mem[ia][7:0];
    rd3 <= mem[ia + 1];
  end
  initial begin
    for (k = 0; k < 8; k = k + 1) mem[k] = k * 16'h1111;
    word = 32'h8000_00f1; sa = -3; sb = 2;
    ia = 2; is = 3; bi = 0;
    #1 $display("%h %h b1=%b b2=%b nb=%b lt=%b", rd1, rd2, b1, b2, nb, lt);
    ia = 9; is = -1; bi = 31; word[7:0] = 0; sa = 5;
    #1 $display("%h %h b1=%b b2=%b nb=%b lt=%b", rd1, rd2, b1, b2, nb, lt);
    ia = 6'bx; bi = 5'bx; sb = 8'bx;
    #1 $display("%h %h b1=%b b2=%b nb=%b lt=%b", rd1, rd2, b1, b2, nb, lt);
    ia = 4; word = 32'h1234_abcd; sb = 5;
    #1 clk = 1; #1 clk = 0;
    $display("mem4=%h word=%h rd3=%h lt=%b", mem[4], word, rd3, lt);
    ia = 7; #1 clk = 1; #1 clk = 0;
    ia = 6'bx; #1 clk = 1; #1 clk = 0;
    ia = 20; #1 clk = 1; #1 clk = 0;
    for (k = 0; k < 8; k = k + 1) $write("%h ", mem[k]);
    $display("word=%h rd3=%h", word, rd3);
  end
endmodule
