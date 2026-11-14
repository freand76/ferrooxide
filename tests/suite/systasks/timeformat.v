// expected
// %t and $timeformat (IEEE 1800 20.4.2, 21.2.1.2): units, precision, suffix, minimum width;
// $time / $realtime / $stime in the module's time unit
`timescale 1ns/1ps
module t;
  initial begin
    #1.234;
    $display("[%t]", $realtime);
    $timeformat(-9, 3, " ns", 12);
    $display("[%t] [%0t]", $realtime, $realtime);
    $timeformat(-6, 6, "us", 0);
    $display("[%t]", $realtime);
    $timeformat(-12, 0, "", 5);
    $display("[%t]", $time);
    $display("%0d %0.3f %0d", $time, $realtime, $stime);
  end
endmodule
