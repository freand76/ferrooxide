// exit: 1
// A ref argument whose variable the subroutine also uses by name: passed by copy here, which
// differs from a reference, so it is rejected.
module t;
  int g;
  function automatic void f(ref int x); x = 1; g = 2; endfunction
  initial f(g);
endmodule
