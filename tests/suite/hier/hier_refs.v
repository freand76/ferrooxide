// Hierarchical names (IEEE 1800 23.6-23.8): downward and upward references, references into
// tasks' and named blocks' variables, hierarchical task calls, $root, %m in each scope
module sub;
  reg [7:0] r = 8'h42;
  task bump; r = r + 1; endtask
  function [7:0] twice(input [7:0] v); twice = 2 * v; endfunction
  initial begin : blk
    reg [3:0] local_v;
    local_v = 4'd7;
  end
  wire [7:0] up = t.top_v;            // upward by module name
endmodule
module t;
  integer errors = 0;
  reg [7:0] top_v = 8'h10;
  sub s1();
  sub s2();
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0h want %0h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    #1;
    check(`__LINE__, s1.r, 8'h42);
    s1.bump;
    check(`__LINE__, s1.r, 8'h43);
    check(`__LINE__, s2.r, 8'h42);      // each instance has its own
    check(`__LINE__, s1.twice(3), 6);
    check(`__LINE__, s1.blk.local_v, 7);
    s2.r = 8'h01;
    check(`__LINE__, s2.r, 1);
    check(`__LINE__, s1.up, 8'h10);
    check(`__LINE__, $root.t.s1.r, 8'h43);
    if (errors == 0) $display("PASSED");
  end
endmodule
