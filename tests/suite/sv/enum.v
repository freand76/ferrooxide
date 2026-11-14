// enums: values, methods (IEEE 1800 6.19)
module t;
  integer errors = 0;
  typedef enum logic [2:0] {RED, GREEN = 3, BLUE} color_t;
  color_t c;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    check(`__LINE__, $bits(color_t), 3);
    check(`__LINE__, BLUE, 4);
    c = c.first();
    check(`__LINE__, c, RED);
    c = c.next();
    check(`__LINE__, c, GREEN);
    c = c.next(2);                // wraps around
    check(`__LINE__, c, RED);
    c = c.prev();
    check(`__LINE__, c, BLUE);
    check(`__LINE__, c.last(), BLUE);
    check(`__LINE__, c.num(), 3);
    if (c.name() != "BLUE") begin $display("FAILED line %0d", `__LINE__); errors++; end
    c = GREEN;
    $display("%s", c.name());
    if (errors == 0) $display("PASSED");
  end
endmodule
