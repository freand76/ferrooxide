class C;
  localparam W = 8;
  typedef logic [W-1:0] T;
  typedef enum { A, B[2] } e_t;
  parameter type P = byte;
  localparam type Q = T, R = int;
  T data;
  e_t st;
endclass
module m;
  C::T x;
  C c;
  C::Q q;
  initial x = C::W + c.W + C::B1 + $bits(C::T) + $size(C::P, 1);
endmodule
