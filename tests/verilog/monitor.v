module tb;
  reg [3:0] a;
  reg b;
  initial begin
    $monitor("%0t a=%d b=%b", $time, a, b);
    a = 0; b = 0;
    #5 a = 3;
    #5 b = 1;
    #1 a = 4; b = 0;
    #1 a = 4;
    #5 $strobe("strobe a=%d", a);
    a = 9;
    #1 $finish;
  end
endmodule
