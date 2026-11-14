// $clog2, $bits, $signed/$unsigned, $random (with a seed: repeatable), $urandom_range bounds,
// $test$plusargs / $value$plusargs
// args: +mode=fast +n=42 +flag
module t;
  integer errors = 0;
  integer seed, x1, x2, i, n;
  reg [8*8:1] mode;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    check(`__LINE__, $clog2(1), 0);
    check(`__LINE__, $clog2(2), 1);
    check(`__LINE__, $clog2(5), 3);
    check(`__LINE__, $clog2(1024), 10);
    check(`__LINE__, $clog2(0), 0);
    check(`__LINE__, $bits(mode), 64);
    seed = 7; x1 = $random(seed);
    seed = 7; x2 = $random(seed);
    check(`__LINE__, x1, x2);
    for (i = 0; i < 100; i++) begin
      n = $urandom_range(3, 9);
      if (n < 3 || n > 9) begin $display("FAILED line %0d: %0d", `__LINE__, n); errors++; end
      n = $urandom_range(9, 3);         // the bounds in either order
      if (n < 3 || n > 9) begin $display("FAILED line %0d: %0d", `__LINE__, n); errors++; end
    end
    check(`__LINE__, $test$plusargs("flag"), 1);
    check(`__LINE__, $test$plusargs("fla"), 1);      // a prefix matches
    check(`__LINE__, $test$plusargs("other"), 0);
    check(`__LINE__, $value$plusargs("n=%d", n), 1);
    check(`__LINE__, n, 42);
    check(`__LINE__, $value$plusargs("mode=%s", mode), 1);
    if (mode != "fast") begin $display("FAILED line %0d", `__LINE__); errors++; end
    check(`__LINE__, $value$plusargs("none=%d", n), 0);
    if (errors == 0) $display("PASSED");
  end
endmodule
