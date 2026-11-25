// exit: 1
// A pure method must be virtual, in a virtual (abstract) class (IEEE 1800-2017 8.21).
module t;
  class C;
    pure virtual function int f();
  endclass
endmodule
