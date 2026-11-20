// exit: 1
// A void function returns no value (IEEE 1800-2017 13.4.1).
module test;
  function void f();
    return 10;
  endfunction
  initial f();
endmodule
