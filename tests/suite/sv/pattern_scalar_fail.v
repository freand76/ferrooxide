// exit: 1
// An assignment pattern needs an array or struct type, not a scalar (IEEE 1800-2017 10.9).
module test;
  bit x = '{1'b1};
endmodule
