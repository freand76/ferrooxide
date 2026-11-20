// exit: 1
// Dynamic arrays assigned to each other need equivalent element types: a 2-state and a
// 4-state element type are not (IEEE 1800-2017 6.22.2, 7.6).
module test;
  logic [7:0] d1[];
  bit [7:0] d2[];
  initial d1 = d2;
endmodule
