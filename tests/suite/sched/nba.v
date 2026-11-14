// Nonblocking assignments (IEEE 1800 4.9.4, 10.4.2): the right side is sampled at once, the
// update happens in the NBA region after the active and inactive events; updates to one
// variable apply in order (the last one wins); `<= #d` schedules a future update.
module t;
  integer errors = 0;
  reg [7:0] a, b, c, d;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0h want %0h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    a = 1; b = 2;
    a <= b; b <= a;               // swap
    check(`__LINE__, a, 1);       // not yet
    #0 check(`__LINE__, a, 1);    // inactive region runs before the NBA region
    #1 check(`__LINE__, a, 2);
    check(`__LINE__, b, 1);
    c <= 1; c <= 2; c <= 3;       // the last one wins
    #1 check(`__LINE__, c, 3);
    d = 5;
    d <= #3 7;
    d = 6;                        // the earlier blocking value stays until time 5
    #2 check(`__LINE__, d, 6);
    #1 check(`__LINE__, d, 6);       // time 5: the update comes in the NBA region
    #1 check(`__LINE__, d, 7);
    // the NBA region runs after every active process of the time step
    #1 c = 0; d = 0;
    fork
      c <= 9;
      begin d = c; end
    join
    check(`__LINE__, d, 0);
    #1 check(`__LINE__, c, 9);
    if (errors == 0) $display("PASSED");
  end
endmodule
