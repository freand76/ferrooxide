// exit: 1
// The base type of an enum is an integer type with at most one packed dimension (IEEE
// 1800-2017 6.19).
module test;
  enum logic [1:0][1:0] {A} e;
endmodule
