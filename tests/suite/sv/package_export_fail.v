// exit: 1
// A wildcard-imported name the package does not refer to is not exported by `*::*`
// (IEEE 1800-2017 26.6), so it cannot be imported from that package.
package p1;
  integer x = 1;
endpackage

package p2;
  import p1::*;
  export *::*;
endpackage

module test;
  import p2::x;
  initial $display("FAILED");
endmodule
