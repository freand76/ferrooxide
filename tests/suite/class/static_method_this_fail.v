// exit: 1
// `C::m()` of a method that is not static needs an object (IEEE 1800-2017 8.10).
module test;
  class C;
    function int f();
      return 1;
    endfunction
  endclass
  int r;
  initial r = C::f();
endmodule
