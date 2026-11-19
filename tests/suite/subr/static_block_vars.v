// `static` variables in blocks of functions and tasks (IEEE 1800-2017 6.21): one variable
// for all calls (from any call site, also of an automatic subroutine), its initializer
// applied once; constant function calls start afresh (13.4.3).
module t;
  integer errors = 0;
  function integer acc_s(input integer value);
    begin : blk
      static int acc = 1;
      acc = acc + value;
      return acc;
    end
  endfunction
  function automatic integer acc_a(input integer value);
    begin : blk
      static int acc = 10;
      automatic int tmp = 100;
      tmp = tmp + value;
      acc = acc + value;
      return acc + tmp;
    end
  endfunction
  task automatic count(output int n);
    begin
      static int calls = 0;
      calls++;
      n = calls;
    end
  endtask
  localparam P1 = acc_s(2);
  localparam P2 = acc_s(3);
  int n;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    check(`__LINE__, P1, 3);
    check(`__LINE__, P2, 4);
    check(`__LINE__, acc_s(2), 3);
    check(`__LINE__, acc_s(3), 6);
    check(`__LINE__, acc_a(1), 112);
    check(`__LINE__, acc_a(1), 113);
    count(n);
    count(n);
    check(`__LINE__, n, 2);
    count(n);
    check(`__LINE__, n, 3);
    if (errors == 0) $display("PASSED");
  end
endmodule
