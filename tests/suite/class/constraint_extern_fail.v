// exit: 1
// an extern constraint has its body after the class (IEEE 1800-2017 18.5.1)
class A;
  rand int x;
  extern constraint c;
endclass
module top;
endmodule
