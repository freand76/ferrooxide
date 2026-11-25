// expected
// A negative delay is a two's-complement time value (IEEE 1364-2005 9.7.1): a real one too,
// scaled to ticks first, so these never fire before the end.
`timescale 1ns/100ps
module top;
  integer d = -1;
  real rd = -2.5;
  reg in = 0;
  wire #d w1 = in;
  wire #rd w2 = in;
  initial begin
    #1 in = 1;
    fork
      #d $display("int delay fired at %0t", $time);
      #rd $display("real delay fired at %0t", $time);
      #(-3) $display("const delay fired at %0t", $time);
      #(-1.5) $display("const real delay fired at %0t", $time);
      @(w1) $display("w1 at %0t", $time);
      @(w2) $display("w2 at %0t", $time);
    join_none
    #100 $display("done %0t %b %b", $time, w1, w2);
    $finish;
  end
endmodule
