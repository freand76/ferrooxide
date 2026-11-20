// exit: 1
// An array of enums takes only an array of the same enum (IEEE 1800-2017 6.22.2, 7.6).
module test;
  typedef enum logic [7:0] { A, B } e_t;
  e_t d1[];
  logic [7:0] d2[];
  initial d1 = d2;
endmodule
