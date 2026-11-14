// expected
// VCD output (IEEE 1800 21.7): $dumpfile/$dumpvars, value changes per time step, vectors with
// x/z, reals, events, $dumpoff (all x) and $dumpon (all values), hierarchy as scopes. The test
// reads its own file back (after the $date/$version header, which names the version).
`timescale 1ns/1ps
module sub(input c, output reg [3:0] q);
  always @(posedge c) q <= q + 1;
  initial q = 0;
endmodule
module t;
  reg clk = 0;
  wire [3:0] q;
  real r = 1.5;
  integer i = 3;
  reg [1:0] x;
  event ev;
  integer fd, n;
  reg [8*80:1] line;
  sub u(clk, q);
  always #5 clk = ~clk;
  initial begin
    $dumpfile("o.vcd");
    $dumpvars;
    #12 x = 2'b1z; r = 2.25; -> ev;
    #10 $dumpoff;
    #10 $dumpon;
    #6 $dumpflush;
    #1;
    fd = $fopen("o.vcd", "r");
    n = 0;
    while ($fgets(line, fd)) begin
      n = n + 1;
      if (n > 6) $write("%0s", line);
    end
    $finish;
  end
endmodule
