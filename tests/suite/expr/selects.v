// Bit and part selects (IEEE 1800 11.5.1): constant part selects, indexed +: / -:, selects
// of little- and big-endian vectors, writes to part selects and out-of-range writes
module t;
  integer errors = 0;
  reg [15:0] v;
  reg [0:15] be;
  reg [7:0] m [0:3];
  integer i;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0h want %0h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    v = 16'hbeef; be = 16'hbeef;
    check(`__LINE__, {v[15:12]}, 4'hb);
    check(`__LINE__, {be[0:3]}, 4'hb);           // the leftmost bits either way
    check(`__LINE__, {v[3]}, 1'b1);
    check(`__LINE__, {be[3]}, 1'b1);             // bit 3 of [0:15] is the 4th from the left
    i = 4;
    check(`__LINE__, {v[i +: 8]}, 8'hee);        // bits 11:4
    check(`__LINE__, {v[i+7 -: 8]}, 8'hee);
    check(`__LINE__, {be[i +: 4]}, 4'he);        // bits 4..7 of [0:15]
    v[i +: 4] = 4'h0;
    check(`__LINE__, v, 16'hbe0f);
    v[15:14] = 2'b00;
    check(`__LINE__, v, 16'h3e0f);
    v[17:14] = 4'b1111;                        // only the bits in range are written
    check(`__LINE__, v, 16'hfe0f);
    v[20] = 1'b0;                              // out of range: no effect
    check(`__LINE__, v, 16'hfe0f);
    i = 'bx;
    v[i] = 1'b0;                               // unknown index: no effect
    check(`__LINE__, v, 16'hfe0f);
    m[1] = 8'h5a;
    check(`__LINE__, {m[1][3:0]}, 4'ha);
    m[1][7:4] = 4'h3;
    check(`__LINE__, {m[1]}, 8'h3a);
    check(`__LINE__, {m[5]}, 8'hxx);             // out of range element reads x
    m[5] = 8'h00;                              // and is not written
    i = 2;
    m[i][i] = 1'b1;
    check(`__LINE__, {m[2][2]}, 1'b1);
    if (errors == 0) $display("PASSED");
  end
endmodule
