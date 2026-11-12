module tb;
  reg [3:0] a, b, c;
  reg clk = 0;
  always @(posedge clk) begin
    a <= b;
    b <= a;
    c <= c + 1;
  end
  initial begin
    a = 1; b = 2; c = 0;
    repeat (4) begin
      #1 clk = 1;
      #1 clk = 0;
      $display("a=%d b=%d c=%d", a, b, c);
    end
    a <= #3 4'd9;
    #2 $display("a=%d", a);
    #2 $display("a=%d", a);
  end
endmodule
