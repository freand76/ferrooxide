// exit: 1
// a constraint body after a class needs a prototype in it (IEEE 1800-2017 18.5.1)
class A;
  rand int x;
endclass
constraint A::c { x > 0; }
module top;
endmodule
