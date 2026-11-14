// File-scope ($unit) and package variables exist once, shared by all modules; P::x names the
// package's variable even when the module has its own x. Unconnected `input var` ports are
// variables (x), `input logic` ports nets (z). Packages are not dumped (with a warning).
bit failed = 1'b0;
package Q; localparam W = 3; endpackage
package P;
  integer T = 10;
  logic [3:0] cnt = 3;
  function void bump(); cnt = cnt + 1; endfunction
endpackage
module m2; import P::*;
  initial #2 begin bump(); $display("m2 cnt=%0d T=%0d failed=%b", cnt, T, failed); end
endmodule
module ports(input logic a, input var b, input wire c, input var logic d, input logic [1:0] e);
  initial #1 $display("ports %b %b %b %b %b", a, b, c, d, e);
endmodule
module test;
  import Q::*;
  typedef integer T;
  T x;
  m2 u();
  ports p();
  initial begin
    $dumpfile("pkg_vars.vcd"); $dumpvars;
    P::bump(); x = P::T; failed = 1;
    #1 P::bump(); $display("cnt=%0d x=%0d W=%0d", P::cnt, x, W);
    #5 $display("cnt=%0d failed=%b", P::cnt, failed);
  end
endmodule
