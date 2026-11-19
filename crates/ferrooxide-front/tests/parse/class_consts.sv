class C;
  localparam W = 8;
  typedef logic [W-1:0] T;
  typedef enum { A, B[2] } e_t;
  T data;
  e_t st;
endclass
module m;
  C::T x;
  C c;
  initial x = C::W + c.W + C::B1;
endmodule
