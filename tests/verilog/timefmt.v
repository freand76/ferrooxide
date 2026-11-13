// $timeformat, $printtimescale, `resetall
`timescale 1ns/10ps
module top;
  reg [3:0] r;
  real t;
  sub s ();
  initial begin
    $printtimescale;
    $printtimescale(s);
    $printtimescale(top.r);
    $printtimescale(r[1]);
    $printtimescale(other);
    #1.25 $display("[%t]", $realtime);
    $timeformat(-9, 2, " ns", 12);
    $display("[%t] [%0t]", $time, $realtime);
    #10 $timeformat(-12, 0, "ps", 1);
    $display("[%t]", $realtime);
    $timeformat;
    $display("[%t]", $time);
  end
endmodule

`timescale 1us/1ns
module sub;
endmodule

`resetall
module other;
endmodule
