// expected
// A real argument without a format prints as %#g (trailing zeros kept); a bare $realtime
// prints with as many decimals as the module's time precision has below its unit.
`timescale 1us/1ps
module top;
  real r;
  initial begin
    r = 1.0; $display(r);
    r = 0.0; $display(r);
    r = 123456789.0; $display(r);
    r = 0.1; $write(r, "\n");
    r = 1e-10; $display("[", r, "]");
    r = -2.5; $display(r, " ", r * 2, " ", 2.0);
    r = 1e30; $display(r);
    #1.25 $display($realtime, " ", $realtime * 2, " ", $time);
    $display("%f %g %0t", $realtime, $realtime, $realtime);
  end
endmodule

`timescale 1ns/100ps
module other;
  initial #1.5 $display($realtime);
endmodule

`timescale 1ns/1ns
module coarse;
  initial #2 $display($realtime);
endmodule
