// expected
// A macro argument default may hold parentheses, brackets, braces and strings with commas
// (IEEE 1800-2017 22.5.1).
`define M(A, B=f(1, 2), C={"x,", "y"}) $display("%0d %0d %s", A, B, C);
module t;
  function int f(int a, int b); return a + b; endfunction
  initial begin
    `M(1)
    `M(1, f(3, 4))
    `M(1, , "z")
  end
endmodule
