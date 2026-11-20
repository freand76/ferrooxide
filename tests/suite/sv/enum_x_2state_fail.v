// exit: 1
// A 2-state enum (default base int) cannot have an x value (IEEE 1800-2017 6.19).
module test;
  enum { A, B = 'bx } e;
endmodule
