// defparam: from the parent, from another top module, into an instance array
module leaf #(parameter W = 4, parameter V = 1) (output [W-1:0] o);
  assign o = V;
  initial #1 $display("%m W=%0d V=%0d", W, V);
endmodule

module mid;
  parameter P = 2;
  leaf #(.W(P)) l1 ();
  leaf l2 ();
  defparam l2.V = P * 3;
endmodule

module top;
  mid m1 ();
  mid m2 ();
  leaf la[1:0] ();
  defparam m1.P = 5, m2.l1.V = 7;
  defparam top.m2.l2.W = 8;
  defparam la[0].V = 9;
  initial #2 $display("%b %b %b", m1.l1.o, m2.l2.o, la[0].o);
endmodule

module other;
  defparam top.m1.l1.V = 3;
endmodule
