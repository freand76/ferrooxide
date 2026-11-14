// Gate primitives with x and z inputs (IEEE 1800 28.4-28.6): z inputs act as x; and/or with
// a controlling value; bufif/notif with an unknown control give L/H-like x; delays.
module t;
  integer errors = 0;
  reg a, b, c;
  wire y_and, y_or, y_xor, y_nand, y_not, y_buf, y_bif1, y_bif0, y_nif1, y_d;
  and  (y_and, a, b, c);
  or   (y_or, a, b);
  xor  (y_xor, a, b);
  nand (y_nand, a, b);
  not  (y_not, a);
  buf  (y_buf, a);
  bufif1 (y_bif1, a, b);
  bufif0 (y_bif0, a, b);
  notif1 (y_nif1, a, b);
  and #3 (y_d, a, b);
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %b want %b", line, got[0], want[0]);
      errors = errors + 1;
    end
  endtask
  initial begin
    a = 1; b = 1; c = 1;
    #5;
    check(`__LINE__, y_and, 1);
    check(`__LINE__, y_d, 1);
    a = 0; b = 1'bx; c = 1;
    #1;
    check(`__LINE__, y_and, 0);              // 0 controls and
    check(`__LINE__, y_or, 1'bx);
    check(`__LINE__, y_xor, 1'bx);
    check(`__LINE__, y_nand, 1);
    check(`__LINE__, y_not, 1);
    check(`__LINE__, y_bif1, 1'bx);          // unknown control
    check(`__LINE__, y_d, 1);                // not yet: 3 time units
    #3 check(`__LINE__, y_d, 0);
    a = 1'bz; b = 1;
    #1;
    check(`__LINE__, y_or, 1);               // 1 controls or
    check(`__LINE__, y_not, 1'bx);           // z input acts as x
    check(`__LINE__, y_buf, 1'bx);
    a = 1; b = 0;
    #1;
    check(`__LINE__, y_bif1, 1'bz);          // disabled
    check(`__LINE__, y_bif0, 1);
    check(`__LINE__, y_nif1, 1'bz);
    b = 1;
    #1 check(`__LINE__, y_nif1, 0);
    if (errors == 0) $display("PASSED");
  end
endmodule
