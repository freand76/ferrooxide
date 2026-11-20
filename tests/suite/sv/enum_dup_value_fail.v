// exit: 1
// The constants of an enum have distinct values (IEEE 1800-2017 6.19).
module test;
  typedef enum logic [1:0] { A, B = 2'd0 } e_t;
  e_t e;
endmodule
