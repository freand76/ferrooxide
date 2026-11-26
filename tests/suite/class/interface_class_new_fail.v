// exit: 1
// no objects of an interface class (IEEE 1800-2017 8.26.5)
module top;
  interface class I;
    pure virtual function void f();
  endclass
  I i;
  initial i = new;
endmodule
