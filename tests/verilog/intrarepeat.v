// Intra-assignment repeat event control; task ports typed by a second declaration
module top;
  reg clk = 0;
  reg [3:0] a, b;
  always #5 clk = ~clk;
  task t;
    input x;
    integer x;
    output [7:0] y;
    reg [7:0] y;
    y = x * 2;
  endtask
  initial begin
    b = 4'd3;
    a = repeat (3) @(posedge clk) b + 1;
    $display("%0t a=%0d", $time, a);
    b = 4'd9;
    a = repeat (0) @(posedge clk) b;
    $display("%0t a=%0d", $time, a);
    t(-5, b);
    $display("%0d %0d", b, $bits(t.x));
    $finish;
  end
endmodule
