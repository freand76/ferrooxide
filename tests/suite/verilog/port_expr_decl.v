// a port declaration may name a signal of a port expression (IEEE 1800-2017 23.2.2.1)
module sub(.p({a, b}), q);
  input a, b;
  output [1:0] q;
  assign q = {a, b};
endmodule
module top;
  wire [1:0] q;
  sub s(.p(2'b10), .q(q));
  initial #1 if (q === 2'b10) $display("PASSED"); else $display("FAILED %b", q);
endmodule
