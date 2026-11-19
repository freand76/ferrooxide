// exit: 1
// A class whose base class is declared after it (with a forward typedef): rejected, not
// silently laid out without the base class's properties.
module test;
  typedef class B;
  class D extends B;
  endclass
  class B;
    int x;
  endclass
  D d;
  initial d = new;
endmodule
