// 4-state semantics (IEEE 1800 11.4): x/z in arithmetic makes the whole result x; bitwise
// operators follow the truth tables; logical operators and ?: with an unknown condition;
// == gives x, === compares x and z exactly; division by zero gives x.
module t;
  integer errors = 0;
  reg [3:0] a, b, r;
  reg c;
  reg [63:0] w64;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %b want %b", line, got[7:0], want[7:0]);
      errors = errors + 1;
    end
  endtask
  initial begin
    a = 4'b10x1; b = 4'b0011;
    check(`__LINE__, {a + b}, 4'bxxxx);
    check(`__LINE__, {a - 4'd1}, 4'bxxxx);
    check(`__LINE__, {a & b}, 4'b00x1);
    check(`__LINE__, {a | b}, 4'b1011);         // x | 1 is 1
    check(`__LINE__, {a ^ b}, 4'b10x0);
    check(`__LINE__, {~a}, 4'b01x0);
    check(`__LINE__, {a & 4'b0z00}, 4'b0000);   // z & 0 and x & 0 are 0
    check(`__LINE__, {a & 4'b1z11}, 4'b10x1);   // z & 0 is 0, x & 1 is x
    check(`__LINE__, {&a}, 1'b0);               // a 0 bit decides
    check(`__LINE__, {|a}, 1'b1);               // a 1 bit decides
    check(`__LINE__, {^a}, 1'bx);
    check(`__LINE__, {a == b}, 1'b0);           // a known differing bit decides
    check(`__LINE__, {a == 4'b1001}, 1'bx);
    check(`__LINE__, {a === 4'b10x1}, 1'b1);
    check(`__LINE__, {a !== 4'b10z1}, 1'b1);
    check(`__LINE__, {a < b}, 1'bx);
    check(`__LINE__, {4'bx && 1'b0}, 1'b0);
    check(`__LINE__, {4'bx || 1'b1}, 1'b1);
    check(`__LINE__, {4'bx && 1'b1}, 1'bx);
    check(`__LINE__, {!4'bx01x}, 1'b0);         // a 1 bit: true, so ! is 0
    c = 1'bx;
    r = c ? 4'b1100 : 4'b1010;                // unknown condition: bits that agree stay
    check(`__LINE__, r, 4'b1xx0);
    check(`__LINE__, {8'd7 / 8'd0}, 8'bxxxxxxxx);
    check(`__LINE__, {8'd7 % 8'd0}, 8'bxxxxxxxx);
    check(`__LINE__, {a << 1}, 4'b0x10);         // shifts move x bits
    check(`__LINE__, {4'b1001 << 2'bx1}, 4'bxxxx);  // an unknown shift count: all x
    check(`__LINE__, {a[1]}, 1'bx);
    check(`__LINE__, {a[4'bx]}, 1'bx);          // an unknown index reads x
    check(`__LINE__, {a[7]}, 1'bx);             // out of range reads x
    check(`__LINE__, {a[5:2]}, 4'bxx10);      // partly out of range
    check(`__LINE__, {2'b1z === 2'b1z}, 1'b1);
    // an unsized constant with a leading x or z extends it to the full width (1364-2005 3.5.1)
    w64 = 'bx;
    check(`__LINE__, w64, 64'bx);
    w64 = 'hz;
    check(`__LINE__, w64, 64'bz);
    w64 = 'bx01;
    check(`__LINE__, w64, {{62{1'bx}}, 2'b01});
    w64 = 'b1x;                                // leading 1: zero-extended
    check(`__LINE__, w64, 64'b1x);
    if (errors == 0) $display("PASSED");
  end
endmodule
