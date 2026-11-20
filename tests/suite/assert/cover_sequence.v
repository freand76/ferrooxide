// expected
// `cover sequence` (16.14.3) runs its pass action for every match of every attempt: an
// attempt starts at each tick, `b[->2]` ends at the second b from its start.
module t;
  reg clk = 0;
  reg a = 0, b = 0;
  //                 k: 9876543210
  reg [9:0] A = 10'b0000101101;
  reg [9:0] B = 10'b0101011010;
  integer i;
  always #5 clk = ~clk;
  cover sequence (@(posedge clk) a ##1 b) $display("%0t a##1b", $time);
  cover sequence (@(posedge clk) a[*2]) $display("%0t a[*2]", $time);
  cover sequence (@(posedge clk) b[->2]) $display("%0t b[->2]", $time);
  cover sequence (@(posedge clk) a ##[1:2] b ##1 !b) $display("%0t a##[1:2]b##1!b", $time);
  initial begin
    for (i = 0; i < 10; i = i + 1) begin
      a = A[i]; b = B[i];
      @(negedge clk);
    end
    a = 0; b = 0;
    #30 $finish;
  end
endmodule
