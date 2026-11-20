module m(input clk, input rst, input a, input b, input [3:0] x);
  sequence s(p, n);
    p ##[1:n] b;
  endsequence
  property q(p);
    disable iff (rst) p |=> s(a, 2);
  endproperty
  default clocking @(posedge clk); endclocking
  a1: assert property (q(a)) else $error("q");
  a2: assert property (@(negedge clk) a[*2] ##1 b[->1:2] or (a intersect b));
  a3: assume property (not (a throughout b[=2]) and (x == 0 |-> $past(x, 2) != 1));
  c1: cover property (if (a) b else $rose(a)) $display("covered");
  cover sequence ((a ##1 b) within (b[*3]));
  restrict property (a);
endmodule
