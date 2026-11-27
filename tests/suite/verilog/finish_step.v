// expected
// $finish ends the simulation at once (IEEE 1800-2017 20.2): the rest of its time step,
// its NBAs and its $monitor / $strobe output do not run; the calling process stops.
module top;
  reg [3:0] a = 0;
  reg [8:0] state = 0;
  wire [8:0] next = state + 1;
  initial $monitor("%0t monitor a=%0d state=%0d next=%0d", $time, a, state, next);
  initial begin
    #1 a = 1;
    $strobe("%0t strobe a=%0d", $time, a);
    a <= 2;
    #0 $display("after #0");
    #9 $finish;
    $display("not reached");
  end
  always #5 state = state + 1;
endmodule
