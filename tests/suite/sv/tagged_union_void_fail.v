// exit: 1
// a void member of a tagged union has no value (IEEE 1800-2017 7.3.2)
module top;
  typedef union tagged { void Invalid; int Valid; } VInt;
  VInt a;
  initial begin
    a = tagged Invalid (1);
  end
endmodule
