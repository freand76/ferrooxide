// Real numbers (IEEE 1800 6.12, 11.3.1, 20.5): real to integer conversion rounds half away
// from zero, $rtoi truncates; an expression with one real operand is real; integer division
// stays integer inside a real expression only for its own operands; $realtobits/$bitstoreal
// round-trip; comparisons, logical use and the conditional operator with reals
module t;
  integer errors = 0;
  real r, s;
  integer i;
  reg [63:0] bits;
  reg [7:0] b;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  task checkr(input integer line, input real got, input real want);
    if (got != want) begin
      $display("FAILED line %0d: got %f want %f", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    i = 2.5;   check(`__LINE__, i, 3);
    i = -2.5;  check(`__LINE__, i, -3);
    i = 2.4;   check(`__LINE__, i, 2);
    i = -0.5;  check(`__LINE__, i, -1);
    i = $rtoi(2.9);  check(`__LINE__, i, 2);
    i = $rtoi(-2.9); check(`__LINE__, i, -2);
    r = 7 / 2;       checkr(`__LINE__, r, 3.0);     // integer division, then converted
    r = 7 / 2.0;     checkr(`__LINE__, r, 3.5);
    r = 1 + 7 / 2 + 0.5;  checkr(`__LINE__, r, 4.5); // 7/2 is integer: 3
    r = $itor(5) / 2;     checkr(`__LINE__, r, 2.5);
    b = 255.6;       check(`__LINE__, b, 0);         // 256 truncated to 8 bits
    r = 8'hff;       checkr(`__LINE__, r, 255.0);    // unsigned
    r = 8'shff;      checkr(`__LINE__, r, -1.0);     // signed
    r = 3.25;
    bits = $realtobits(r);
    s = $bitstoreal(bits);
    checkr(`__LINE__, s, 3.25);
    check(`__LINE__, bits, 64'h400a000000000000);
    check(`__LINE__, r > 3, 1);
    check(`__LINE__, r == 3.25, 1);
    check(`__LINE__, !r, 0);
    check(`__LINE__, 0.0 || 0, 0);
    r = r > 3 ? 1.5 : 2;  checkr(`__LINE__, r, 1.5);
    r = 2 ** 0.5;    if (r < 1.4142 || r > 1.4143) begin $display("FAILED line %0d", `__LINE__); errors++; end
    r = -r;          check(`__LINE__, r < 0, 1);
    r = 1.0e3 + 1e-3; checkr(`__LINE__, r, 1000.001);
    r = 1_000.5;     checkr(`__LINE__, r, 1000.5);
    i = 3; r = i;    checkr(`__LINE__, r, 3.0);
    r = $sqrt(16.0); checkr(`__LINE__, r, 4.0);
    r = $pow(2.0, 3.0); checkr(`__LINE__, r, 8.0);
    r = $floor(-1.5); checkr(`__LINE__, r, -2.0);
    r = $ceil(1.2);  checkr(`__LINE__, r, 2.0);
    r = $ln(1.0);    checkr(`__LINE__, r, 0.0);
    if (errors == 0) $display("PASSED");
  end
endmodule
