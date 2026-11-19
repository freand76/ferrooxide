// exit: 1
// `C::new` gives an object of class C, which a handle of a derived class cannot hold
// (IEEE 1800-2017 8.8).
module test;
  class B;
  endclass
  class D extends B;
  endclass
  D d;
  initial d = B::new;
endmodule
