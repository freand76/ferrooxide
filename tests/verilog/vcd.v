module sub(input clk, input [3:0] d, output reg [3:0] q);
  always @(posedge clk) q <= d;
endmodule
module tb;
  reg clk = 0;
  reg [3:0] d = 0;
  wire [3:0] q;
  sub s(.clk(clk), .d(d), .q(q));
  always #5 clk = ~clk;
  initial begin
    $dumpfile("vcd_test.vcd");
    $dumpvars(0, tb);
    repeat (4) begin @(negedge clk) d = d + 3; end
    #3 $display("done q=%d", q);
    $finish;
  end
endmodule
