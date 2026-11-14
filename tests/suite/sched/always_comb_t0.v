// always_comb / always_latch run once at time 0 after all initial and always procedures have
// started (IEEE 1800 9.2.2.2.2); always @* waits for a change.
module t;
  integer errors = 0, runs_comb = 0, runs_star = 0;
  reg [3:0] s, k;
  reg [3:0] y, y2, l, z;
  always_comb begin
    runs_comb = runs_comb + 1;
    y = s + 1;
  end
  always_comb y2 = k;           // k is set before the time-0 run and never changes
  always_latch if (k == 3) l = 4'd7;
  always @* begin
    runs_star = runs_star + 1;
    z = k;
  end
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0h want %0h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    s = 2; k = 3;
    #1;
    check(`__LINE__, y, 3);
    check(`__LINE__, y2, 3);
    check(`__LINE__, l, 7);
    check(`__LINE__, z, 3);       // always @* woke on the change of k
    check(`__LINE__, runs_star, 1);
    s = 5;
    #1 check(`__LINE__, y, 6);
    if (runs_comb < 2) begin
      $display("FAILED: always_comb ran %0d times", runs_comb);
      errors = errors + 1;
    end
    if (errors == 0) $display("PASSED");
  end
endmodule
