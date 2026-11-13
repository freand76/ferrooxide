// A net declaration delay (wire #4 y = x;) is inertial
module netdelay;
  reg x;
  wire #4 y = x;
  initial begin
    x = 0;
    #10 x = 1;
    #2 x = 0;
    #10 x = 1;
    #10 $display("netdelay done");
  end
  initial $monitor("%0t y=%b x=%b", $time, y, x);
endmodule
