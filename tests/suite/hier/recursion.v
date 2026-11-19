// A module instantiates itself through a generate block until a parameter ends the
// recursion: a tree summing 2**L values.
module sum #(parameter L = 3) (input [8*(1<<L)-1:0] v, output [8+L-1:0] s);
  if (L == 0) begin : leaf
    assign s = v;
  end else begin : node
    wire [8+L-2:0] a, b;
    sum #(L-1) lo(v[8*(1<<(L-1))-1:0], a);
    sum #(L-1) hi(v[8*(1<<L)-1:8*(1<<(L-1))], b);
    assign s = a + b;
  end
endmodule

module test;
  wire [10:0] s;
  sum #(3) t({8'd1, 8'd2, 8'd3, 8'd4, 8'd5, 8'd6, 8'd7, 8'd255}, s);
  initial begin
    #1;
    if (s === 283) $display("PASSED");
    else $display("FAILED: s = %0d", s);
  end
endmodule
