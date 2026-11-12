// `timescale: per-module units, fractional delays rounded to the precision, $time/$realtime/%t,
// and the directive staying in effect for later modules (and files).
`timescale 1ns / 1ps
module child;
  initial begin
    #2.5 $display("child: time=%0t $time=%0d $realtime=%f", $time, $time, $realtime);
    #(1.2345) $display("child: %t %0d", $realtime, $time);
  end
endmodule

`timescale 1us / 1ns
module tb;
  child c();
  reg clk = 0;
  integer n = 0;
  initial begin
    #1.5;
    $display("tb: time=%0t $time=%0d $realtime=%f", $time, $time, $realtime);
    #0.0004 $display("tb: %t", $realtime);
    repeat (5) #0.0333 n = n + 1;
    $display("tb: n=%0d %t %0d %f", n, $realtime, $stime, $realtime);
    $display("%t|%0t|%d", $time, $time, $time);
  end
endmodule
