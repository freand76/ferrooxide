// iverilog: -g2012 -Ptop.W=16 -Ptop.NAME="abc" -Ptop.R=2.5
// -P<top>.<param>=<value> sets parameters of top-level modules: a number, a real or a
// string; parameters that depend on them follow.
module top #(parameter int W = 8, parameter int D = W * 2, parameter NAME = "default",
             parameter real R = 1.0, parameter KEEP = 3) ();
  initial $display("W=%0d D=%0d NAME=%s R=%f KEEP=%0d", W, D, NAME, R, KEEP);
endmodule
