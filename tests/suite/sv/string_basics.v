// strings (IEEE 1800 6.16): length, character reads and writes by index, comparison
module t;
  integer errors = 0;
  string s = "that is a test";
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    check(`__LINE__, s.len(), 14);
    check(`__LINE__, $size(s), 14);
    check(`__LINE__, s[0], "t");
    for (int i = 0; i < $size(s); ++i)
      if (s[i] == "t") s[i] = "w";
    if (s != "whaw is a wesw") begin $display("FAILED line %0d: %s", `__LINE__, s); errors++; end
    check(`__LINE__, s < "whax", 1);
    if (errors == 0) $display("PASSED");
  end
endmodule
