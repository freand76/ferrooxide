// expected
// A nonblocking assignment with an event control does not block the process: the value is
// taken at the statement, the update waits for the event (counted from the statement), and
// several such updates can be pending at once.
module top;
  reg [7:0] vec;
  reg [3:0] q;
  reg clk = 0;
  integer off, i;
  event trig;
  initial begin
    vec = 8'hff;
    vec[1] <= @(trig) 1'b0;
    $display("after nba");
    ->trig;
    #1 $display("%h", vec);
    off = 2;
    vec[off] <= @(trig) 1'b0;
    ->trig;
    #1 $display("%h", vec);
    for (i = 0; i < 3; i = i + 1)
      vec[i + 4] <= @(trig) 1'b0;
    #1 $display("%h", vec);
    ->trig;
    #1 $display("%h", vec);
    q = 0;
    q <= repeat (2) @(posedge clk) 4'd9;
    q <= @(posedge clk) 4'd5;
    #1 $display("q=%0d", q);
  end
  always #2 clk = ~clk;
  always @(q) $display("%0t q=%0d", $time, q);
  initial #20 $finish;
endmodule
