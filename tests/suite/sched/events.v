// Edges (IEEE 1800 9.4.2: 0->x/z and x/z->1 are posedges, 1->x/z and x/z->0 negedges),
// named events, wait(), and level-sensitive event controls.
module t;
  integer errors = 0, pos = 0, neg = 0, any = 0, evs = 0;
  reg c;
  reg [3:0] v;
  event e;
  always @(posedge c) pos = pos + 1;
  always @(negedge c) neg = neg + 1;
  always @(v) any = any + 1;
  always @e evs = evs + 1;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    #1 c = 0;                    // x->0: negedge
    #1 c = 1'bz;                 // 0->z: posedge
    #1 c = 1;                    // z->1: posedge
    #1 c = 1'bx;                 // 1->x: negedge
    #1 c = 0;                    // x->0: negedge
    #1 c = 1;                    // 0->1: posedge
    #1 check(`__LINE__, pos, 3);
    check(`__LINE__, neg, 3);
    v = 1; #1 v = 1; #1 v = 2;   // writing the same value is no change
    #1 check(`__LINE__, any, 2);
    -> e; #1 -> e; #1 check(`__LINE__, evs, 2);
    fork
      begin wait (v == 7); check(`__LINE__, $time, 20); end
      #8 v = 7;
    join
    wait (v == 7);               // already true: no wait
    check(`__LINE__, $time, 20);
    if (errors == 0) $display("PASSED");
  end
endmodule
