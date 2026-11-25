// exit: 1
// A ref argument needs a variable (IEEE 1800-2017 13.5.2).
module t;
  function automatic void f(ref int x); x = 1; endfunction
  initial f(3);
endmodule
