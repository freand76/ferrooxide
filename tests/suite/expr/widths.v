// Expression bit lengths (IEEE 1800 11.6, 1364-2005 5.4): context-determined operands are
// widened to the largest width of the expression and the target before evaluation;
// comparisons, reductions and concatenation operands are self-determined.
module t;
  integer errors = 0;
  reg [3:0] a, b;
  reg [7:0] r8;
  reg [4:0] r5;
  reg [15:0] r16;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0h want %0h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    a = 4'hf; b = 4'h1;
    r8 = a + b;                       // evaluated in 8 bits: no carry lost
    check(`__LINE__, r8, 8'h10);
    r5 = a + b;
    check(`__LINE__, r5, 5'h10);
    r8 = (a + b) >> 1;                // the target widens the sum first: 16 >> 1
    check(`__LINE__, r8, 8'h08);
    r8 = {a + b};                     // concatenation operands are self-determined
    check(`__LINE__, r8, 8'h00);
    check(`__LINE__, {a + b == 5'h10}, 1);    // comparison operands: max of both sides
    check(`__LINE__, {(a + b) == 4'd0}, 1);   // ... both 4 bits here: 0
    r8 = ~a;                          // ~ is context-determined: widened first
    check(`__LINE__, r8, 8'hf0);
    r8 = !a;                          // logical not: 1 bit
    check(`__LINE__, r8, 8'h00);
    r8 = &a;
    check(`__LINE__, r8, 8'h01);
    r16 = a * a;
    check(`__LINE__, r16, 16'd225);
    r8 = a << 4;                      // shift count self-determined, the left side widened
    check(`__LINE__, r8, 8'hf0);
    r8 = 4'hf << 4'd4;
    check(`__LINE__, r8, 8'hf0);
    r8 = a ? b : 8'hff;               // ?: operands widened to the larger
    check(`__LINE__, r8, 8'h01);
    check(`__LINE__, {$bits({a, b, 3'b0})}, 11);
    check(`__LINE__, {$bits({3{a}})}, 12);
    r16 = {2{a, b}};
    check(`__LINE__, r16, 16'hf1f1);
    r8 = 1'b1 + 1'b1;                 // widened to 8 bits: 2
    check(`__LINE__, r8, 2);
    r8 = 'hfff;                       // an unsized constant is at least 32 bits, truncated
    check(`__LINE__, r8, 8'hff);
    r16 = 8'hff + 8'h01;
    check(`__LINE__, r16, 16'h100);
    if (errors == 0) $display("PASSED");
  end
endmodule
