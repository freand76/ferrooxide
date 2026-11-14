// Further operators (IEEE 1800 11.4): power, logical/arithmetic shifts, reduction, the
// conditional operator, replication with a zero count inside a concatenation, ==? / !=?.
// Integer results go through an integer variable (signed, 32 bits).
module t;
  integer errors = 0;
  integer i, k;
  reg [7:0] r;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0h want %0h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    i = 2 ** 10;     check(`__LINE__, i, 1024);
    i = 3 ** 0;      check(`__LINE__, i, 1);
    i = 0 ** 0;      check(`__LINE__, i, 1);
    i = (-2) ** 3;   check(`__LINE__, i, -8);
    i = 2 ** -1;     check(`__LINE__, i, 0);       // a negative exponent: 0 unless the base is +-1
    i = (-1) ** -3;  check(`__LINE__, i, -1);
    i = 0 ** -1;     check(`__LINE__, i, 'bx);
    check(`__LINE__, {8'b1 << 9}, 8'b0);
    check(`__LINE__, {8'h80 >> 7}, 8'd1);
    k = -16;
    i = k >>> 2;     check(`__LINE__, i, -4);
    i = k >> 28;     check(`__LINE__, i, 15);
    check(`__LINE__, {~&8'hff}, 1'b0);
    check(`__LINE__, {~|8'h00}, 1'b1);
    check(`__LINE__, {~^8'h03}, 1'b1);
    r = 8'h0f;
    check(`__LINE__, {r[0] ? 8'haa : r[7] ? 8'hbb : 8'hcc}, 8'haa);
    check(`__LINE__, {4'h5, {0{1'b1}}, 4'ha}, 8'h5a);
    check(`__LINE__, {4'b1010 ==? 4'b1x1z}, 1'b1);
    check(`__LINE__, {4'b1011 ==? 4'b1x10}, 1'b0);
    check(`__LINE__, {4'b1x11 ==? 4'b1011}, 1'bx);  // x on the left is not a wildcard
    check(`__LINE__, {4'b1010 !=? 4'b0xxx}, 1'b1);
    k = 5;
    i = -k;          check(`__LINE__, i, -5);
    i = k - -k;      check(`__LINE__, i, 10);
    check(`__LINE__, {{8{1'b1}} + 9'd1}, 9'h100);
    if (errors == 0) $display("PASSED");
  end
endmodule
