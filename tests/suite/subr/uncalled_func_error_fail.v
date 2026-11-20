// exit: 1
// A function that is never called is still checked: its errors are reported.
module test;
  function int f(input int a);
    return a + undeclared_name;
  endfunction
endmodule
