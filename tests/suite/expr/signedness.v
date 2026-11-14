// Signedness (IEEE 1800 11.8.1): an expression is signed only if all operands are; unsigned
// operands make the whole expression unsigned; $signed / $unsigned; sign extension happens
// when a signed operand is widened.
module t;
  integer errors = 0;
  reg signed [3:0] s;
  reg [3:0] u;
  reg signed [7:0] s8;
  reg [7:0] u8;
  integer i;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0h want %0h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    s = -2; u = 2;
    s8 = s;                           // sign extended
    check(`__LINE__, {s8}, 8'hfe);
    u8 = s;                           // still sign extended: the source is signed
    check(`__LINE__, u8, 8'hfe);
    s8 = s + u;                       // unsigned: s zero-extended: 14 + 2
    check(`__LINE__, s8, 8'h10);
    s8 = s + $signed(u);              // both signed: -2 + 2
    check(`__LINE__, s8, 8'h00);
    check(`__LINE__, {s < 0}, 1);
    check(`__LINE__, {s < u}, 0);       // unsigned compare: 14 < 2 false
    check(`__LINE__, {s < $signed(u)}, 1);
    i = s;                            // integer is signed: -2
    check(`__LINE__, i, -2);
    i = s * 3;                        // signed * signed constant
    check(`__LINE__, i, -6);
    i = s * 3'd3;                     // sized decimal constant is unsigned
    check(`__LINE__, {i[3:0]}, 4'ha);   // 14 * 3 = 42 = 'h2a
    check(`__LINE__, i, 42);
    s8 = -8'sd5 >>> 1;                // arithmetic shift keeps the sign
    check(`__LINE__, {s8}, 8'hfd);
    u8 = 8'hf6 >>> 1;                 // unsigned operand: logical
    check(`__LINE__, u8, 8'h7b);
    s8 = -5 / 2;                      // signed division truncates toward zero
    check(`__LINE__, {s8}, 8'hfe);
    s8 = -5 % 2;                      // remainder takes the sign of the first operand
    check(`__LINE__, {s8}, 8'hff);
    i = $unsigned(-1) >> 28;          // 32-bit unsigned
    check(`__LINE__, i, 15);
    check(`__LINE__, {-4'sd1 == 4'b1111}, 1);
    check(`__LINE__, {$signed(4'b1000) < 0}, 1);
    s8 = {s};                         // a concatenation is unsigned: zero-extended
    check(`__LINE__, s8, 8'h0e);
    s8 = s[3:0];                      // a part select is unsigned
    check(`__LINE__, s8, 8'h0e);
    if (errors == 0) $display("PASSED");
  end
endmodule
