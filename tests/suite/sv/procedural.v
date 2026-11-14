// SystemVerilog procedural statements: ++/--/op= (IEEE 1800 11.4.1-2), do-while, break,
// continue, loop variables declared in for (12.7)
module t;
  integer errors = 0;
  int a, n;
  logic [7:0] b;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    a = 1; a++; a += 3; a <<= 2; a--; a -= 1; a *= 3;
    check(`__LINE__, a, 54);
    a /= 4; a %= 5; a |= 8; a &= 12; a ^= 1;
    check(`__LINE__, a, 9);
    b = 8'hff; b++;
    check(`__LINE__, b, 0);
    a = 0;
    do a = a + 2; while (a < 7);
    check(`__LINE__, a, 8);
    do a++; while (0);            // the body runs once
    check(`__LINE__, a, 9);
    n = 0;
    for (int i = 0; i < 100; i++) begin
      if (i % 2) continue;
      if (i == 10) break;
      n += i;
    end
    check(`__LINE__, n, 20);      // 0+2+4+6+8
    a = 0;
    while (1) begin
      a++;
      if (a == 4) break;
    end
    check(`__LINE__, a, 4);
    if (errors == 0) $display("PASSED");
  end
endmodule
