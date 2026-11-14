// Delays: intra-assignment delays sample the right side first (IEEE 1800 10.4.1); time units
// and rounding to the precision (3.14.2); continuous assignment delays are inertial (10.3.3).
`timescale 1ns/100ps
module t;
  integer errors = 0;
  reg [7:0] a, b;
  reg p;
  wire #2 q = p;
  realtime r;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    a = 1;
    b = #2 a;                     // samples a = 1 at time 0
    check(`__LINE__, b, 1);
    check(`__LINE__, $time, 2);
    fork
      #1 a = 9;
      begin b = #2 a; end
    join
    check(`__LINE__, b, 1);       // sampled before a changed
    #1.26;                        // rounds to 1.3 ns
    r = $realtime;
    if (r != 5.3) begin $display("FAILED line %0d: %f", `__LINE__, r); errors = errors + 1; end
    check(`__LINE__, $time, 5);   // $time rounds to the unit
    p = 0;
    #3 p = 1;
    #1 p = 0;                     // a 1 ns pulse is shorter than the 2 ns delay: filtered
    #0.5 check(`__LINE__, q, 0);
    #1 check(`__LINE__, q, 0);
    p = 1;
    #3 check(`__LINE__, q, 1);
    if (errors == 0) $display("PASSED");
  end
endmodule
