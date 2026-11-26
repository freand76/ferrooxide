// exit: 1
// an implementation has the signature of its prototype (IEEE 1800-2017 8.26.6.1)
module top;
  interface class I;
    pure virtual function int f(int a);
  endclass
  class C implements I;
    virtual function int f(logic [7:0] a);
      return a;
    endfunction
  endclass
endmodule
