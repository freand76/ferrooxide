// expected
// $strobe prints at the end of the time step (after nonblocking updates), $display at once;
// $monitor prints when an argument changes, once per time step; $monitoroff/$monitoron
module t;
  reg [3:0] a, b;
  initial begin
    a = 0; b = 0;
    $monitor("%0t monitor a=%0d b=%0d", $time, a, b);
    #1 a <= 1;
    $display("%0t display a=%0d", $time, a);
    $strobe("%0t strobe a=%0d", $time, a);
    #1 a = 2; a = 3; b = 1;              // one monitor line with the final values
    #1 $monitoroff;
    a = 4;
    #1 $monitoron;                       // prints the current values again
    #1 b = 2;
    #1 $finish;
  end
endmodule
