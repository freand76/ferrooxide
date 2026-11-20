// exit: 1
// Class methods have automatic lifetime; `static` lifetime is an error (IEEE 1800-2017 8.6).
module test;
  class C;
    function static int f(int x);
      return x;
    endfunction
  endclass
  initial $display("FAILED");
endmodule
