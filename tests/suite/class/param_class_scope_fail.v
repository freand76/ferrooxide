// exit: 1
// Outside a parameterized class, `C::x` needs parameter values (IEEE 1800-2017 8.25.1).
module test;
  class par_cls #(int a = 25);
    parameter int b = 23;
  endclass
  initial $display(par_cls::b);
endmodule
