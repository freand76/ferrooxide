// exit: 1
// A name used through a wildcard import may not be declared in the module
// afterwards (IEEE 1800-2017 26.3).
package p;
  integer x = 1;
endpackage

module test;
  import p::*;
  initial $display("FAILED %0d", x);
  integer x = 2;
endmodule
