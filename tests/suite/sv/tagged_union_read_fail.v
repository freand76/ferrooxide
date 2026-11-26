// exit: 1
// reading a member of a tagged union while the tag names another member is a run-time
// error (IEEE 1800-2017 7.3.2)
module top;
  typedef union tagged { void Invalid; int Valid; } VInt;
  VInt a;
  int c;
  initial begin
    a = tagged Invalid;
    c = a.Valid;
    $display("PASSED");
  end
endmodule
