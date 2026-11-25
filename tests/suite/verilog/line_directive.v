// expected
// `line N "file" level: the next line is line N of that file for `__LINE__ and `__FILE__,
// also inside macros.
module top;
`define where(x) $display("%0d -> %s:%0d", x, `__FILE__, `__LINE__)
  initial begin
`line 1000 "first.v" 0
    $display("%0d", `__LINE__);
    `where(1);
`line 20 "second.v" 1

    `where(2);
  end
endmodule
