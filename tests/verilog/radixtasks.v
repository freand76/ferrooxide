// Default-radix variants of the display tasks; %E/%G; infinite reals with %d
module top;
  reg [4:0] v;
  reg [15:0] s;
  real z;
  initial begin
    v = 10; z = 0.0;
    $fdisplay(1, "v ", v);
    $fdisplayb(1, "v ", v);
    $fdisplayo(1, "v ", v);
    $fwriteh(1, "v ", v, "\n");
    $strobeb("strobe ", v);
    $swriteh(s, v);
    $display("%s", s);
    $display("%G %E %d %d", 1.5e9, 2.25e-3, 1.0/z, -1.0/z);
    #1 $monitorh("mon ", v);
    #1 v = 31;
  end
endmodule
