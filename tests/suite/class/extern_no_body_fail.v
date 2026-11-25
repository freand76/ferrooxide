// exit: 1
// An extern method needs its body outside the class (IEEE 1800-2017 8.24).
module t;
  class C;
    extern function int f();
  endclass
  initial begin C c = new; end
endmodule
