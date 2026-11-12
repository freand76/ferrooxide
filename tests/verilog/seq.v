module counter #(parameter W = 4) (input clk, input rst, output reg [W-1:0] q);
  always @(posedge clk or posedge rst)
    if (rst) q <= 0;
    else q <= q + 1'b1;
endmodule

module tb;
  reg clk = 0, rst = 1;
  wire [3:0] q4;
  wire [7:0] q8;
  counter c4(.clk(clk), .rst(rst), .q(q4));
  counter #(.W(8)) c8(clk, rst, q8);
  always #5 clk = ~clk;
  initial begin
    #12 rst = 0;
    repeat (20) @(posedge clk) $display("%0t q4=%d q8=%d", $time, q4, q8);
    $finish;
  end
endmodule
