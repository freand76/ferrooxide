// expected
// unsized x/z constants extend their leading x/z to the width of the expression
module t #(parameter [63:0] P = 'bx);
  localparam Q = 'hz;
  reg [63:0] q; reg [47:0] r;
  initial begin
    $display("%h", P);
    q = Q; $display("%h", q);
    q = 'bx01; $display("%b", q[35:0]);
    q = 'dx; $display("%h", q);
    q = 'dz; $display("%h", q);
    q = 'b1x; $display("%h", q);
    q = 'sbx; $display("%h", q);
    r = 'h?; $display("%h", r);
    q = 'bx + 1'b0; $display("%h", q);
  end
endmodule
