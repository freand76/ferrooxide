// exit: 1
// a member of a tagged union is set only by a tagged union expression (IEEE 1800-2017 7.3.2)
module top;
  typedef union tagged { void Invalid; int Valid; } VInt;
  VInt a;
  initial a.Valid = 3;
endmodule
