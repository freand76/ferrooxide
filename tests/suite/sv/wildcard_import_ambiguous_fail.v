// exit: 1
// A name that two wildcard imports provide is ambiguous where it is used
// (IEEE 1800-2017 26.3).
package p1;
  integer x = 1;
endpackage

package p2;
  integer x = 2;
endpackage

module test;
  import p1::*;
  import p2::*;
  initial $display("FAILED %0d", x);
endmodule
