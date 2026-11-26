// exit: 1
// a class implements every method of its interface classes (IEEE 1800-2017 8.26.2)
module top;
  interface class I;
    pure virtual function void f();
    pure virtual function void g();
  endclass
  class C implements I;
    virtual function void f();
    endfunction
  endclass
endmodule
