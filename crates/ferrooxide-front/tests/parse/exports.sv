package p1;
  integer x = 1;
  typedef logic [3:0] nib_t;
endpackage

package p2;
  import p1::x, p1::nib_t;
  export p1::x, p1::nib_t;
  export p1::*;
  export *::*;
endpackage

module m;
  import p2::*;
  nib_t n;
  initial n = x;
endmodule
