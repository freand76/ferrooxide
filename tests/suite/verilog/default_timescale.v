// A module without `timescale or timeunit has 1ns / 1ns (IEEE 1800-2017 3.14.2.3 leaves
// the default to the tool): a time literal counts in nanoseconds, a fractional delay
// rounds to whole nanoseconds.
module top;
  realtime t0, t1, t2;
  initial begin
    #10ns t0 = $realtime;
    #1.5 t1 = $realtime;
    #2us t2 = $realtime;
    if (t0 == 10 && t1 == 12 && t2 == 2012) $display("PASSED");
    else $display("FAILED: %0f %0f %0f", t0, t1, t2);
  end
endmodule
