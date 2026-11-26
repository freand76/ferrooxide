// exit: 1
// a class that is not virtual overrides the pure constraints of its base (IEEE 1800-2017 18.5.2)
virtual class A;
  pure constraint c;
endclass
class B extends A;
endclass
module top;
endmodule
