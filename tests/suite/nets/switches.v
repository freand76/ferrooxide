// Bidirectional switches (IEEE 1800 28.8): tran connects two nets both ways; tranif1 /
// tranif0 only while the control is 1 / 0; rtran keeps the value but reduces strength.
module t;
  integer errors = 0;
  reg da, db, en;
  wire a, b, c, d;
  assign a = da;
  assign b = db;
  tran (a, c);                // c follows a
  tranif1 (b, d, en);
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %b want %b", line, got[0], want[0]);
      errors = errors + 1;
    end
  endtask
  initial begin
    da = 1; db = 0; en = 0;
    #1;
    check(`__LINE__, c, 1);
    check(`__LINE__, d, 1'bz);
    en = 1;
    #1 check(`__LINE__, d, 0);
    db = 1'bz; da = 1'bz;
    #1;
    check(`__LINE__, a, 1'bz);
    check(`__LINE__, d, 1'bz);
    if (errors == 0) $display("PASSED");
  end
endmodule
