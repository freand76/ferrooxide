// fork/join, join_any, join_none, disable fork (IEEE 1800 9.3.2, 9.6.3)
module t;
  integer errors = 0;
  reg [7:0] a, b;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    fork
      #3 a = 1;
      #5 b = 2;
    join
    check(`__LINE__, $time, 5);
    fork
      #3 a = 3;
      #7 b = 4;
    join_any
    check(`__LINE__, $time, 8);
    check(`__LINE__, a, 3);
    check(`__LINE__, b, 2);       // still running
    #4 check(`__LINE__, b, 4);
    fork
      #2 a = 5;
    join_none
    check(`__LINE__, $time, 12);  // join_none does not wait
    check(`__LINE__, a, 3);
    #3 check(`__LINE__, a, 5);
    fork
      #2 a = 6;
      #4 b = 7;
    join_any
    disable fork;                 // ends the branch still running
    #10 check(`__LINE__, b, 4);
    check(`__LINE__, a, 6);
    if (errors == 0) $display("PASSED");
  end
endmodule
