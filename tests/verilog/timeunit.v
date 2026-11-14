// timeunit / timeprecision at file scope and in modules, time literals, and a file-scope task
// that runs with the file scope's time unit.
timeunit 100ps / 10ps;

task delay(output [63:0] t);
  begin
    $printtimescale;
    #5ns t = $time;
  end
endtask

module fast(output reg out);
  timeunit 10us;
  timeprecision 1us;
  initial begin
    $printtimescale;
    #0 out = 0;
    #1 out = 1;
  end
endmodule

module dflt(output reg out);
  initial begin
    $printtimescale;
    #0 out = 0;
    #1 out = 1;
  end
endmodule

module top;
  timeunit 1ns / 1ps;
  reg [63:0] t1, t2;
  wire f, d;
  realtime r;
  fast m1(f);
  dflt m2(d);
  initial begin
    $printtimescale;
    delay(t1);
    t2 = $time;
    $display("%0d %0d", t1, t2);
    #33.1ns r = $realtime; $display("%f", r);
    #78.1ps r = $realtime; $display("%f", r);
    #1.5us $display("%0t f=%b d=%b", $time, f, d);
    #10us $display("%0t f=%b d=%b", $time, f, d);
  end
endmodule
