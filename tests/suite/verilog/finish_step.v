// expected
// $finish ends the simulation after the current time step's other scheduled processes, its
// NBAs and its $monitor / $strobe output (as in the reference); the calling process stops.
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
