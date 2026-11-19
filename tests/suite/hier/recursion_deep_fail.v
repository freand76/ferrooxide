// exit: 1
// A recursion through a generate block that never ends is cut off after 10 levels.
module r #(parameter N = 0) ();
  if (N >= 0) begin : g
    r #(N + 1) next();
  end
endmodule

module test;
  r top();
endmodule
