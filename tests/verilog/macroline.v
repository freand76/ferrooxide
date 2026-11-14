// `__LINE__ inside macro bodies is the line of the (outermost) invocation
`define L `__LINE__
`define CHECK(x) if (!(x)) $display("line %0d: %s failed", `__LINE__, `"x`");
`define TWICE `L + `L
module t;
  initial begin
    $display("%0d", `__LINE__);
    $display("%0d", `L);
    $display("%0d", `TWICE);
    `CHECK(1 == 2)
    `CHECK(1 == 1)
  end
endmodule
