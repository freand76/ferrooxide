module tb;
  reg [7:0] a, b;
  wire [8:0] sum;
  reg clk;
  integer i;
  assign sum = a + b;
  initial begin
    a = 8'd10; b = 8'd20;
    #1 $display("sum=%d a=%h b=%b", sum, a, b);
    a = 8'hff; b = 8'h01;
    #1 $display("sum=%d %0d %h", sum, sum, sum);
    for (i = 0; i < 4; i = i + 1) begin
      a = i * 3;
      #1 $display("i=%0d a=%d sum=%0d", i, a, sum);
    end
    $display("%d %d", -5, 8'sd200);
    $display(a, b);
    $display("time=%t now=%0t", $time, $time);
    $finish;
  end
endmodule
