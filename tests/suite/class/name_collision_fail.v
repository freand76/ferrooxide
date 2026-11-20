// exit: 1
// An enum constant and a typedef of the same name in one class (IEEE 1800-2017 8.3).
module test;
  class C;
    enum { A = 10 } e;
    typedef int A;
  endclass
  initial $display("FAILED");
endmodule
