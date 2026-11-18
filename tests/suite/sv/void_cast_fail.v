// exit: 1
// A void function has no value to cast (IEEE 1800-2017 13.4.1).
module test;
  function void f(int x);
  endfunction
  initial void'(f(10));
endmodule
