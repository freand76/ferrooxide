// ?: with an unknown condition: bits where both sides agree are kept (also z)
module top;
  reg s;
  reg [3:0] a, b;
  wire [3:0] w = s ? a : b;
  wire n = s ? 1'bz : 1'bz;
  initial begin
    s = 1'bx;
    a = 4'b10zx; b = 4'b11zx;
    #1 $display("%b %b %b", w, n, s ? 4'bz01x : 4'bz11x);
`ifdef __ICARUS__
    $display("icarus");
`endif
  end
endmodule
