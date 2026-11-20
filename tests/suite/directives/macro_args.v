// Macro arguments (IEEE 1800-2017 22.5.1): empty arguments, defaults for missing ones, `()`
// on a macro without parameters, stringification with `", and a valid `line.
`define PAIR(a, b) {a, b}
`define DFLT(a = 4'h1, b = 4'h2) {a, b}
`define NONE() 8'h77
`define STR(x) `"x`"
module t;
  initial begin
`line 100 "macro_args.v" 0
    if (`PAIR(4'h3, 4'h4) === 8'h34 && `DFLT() === 8'h12 && `DFLT(4'h5) === 8'h52
        && `NONE() === 8'h77 && `STR(abc) == "abc" && `__LINE__ == 101)
      $display("PASSED");
    else
      $display("FAILED");
  end
endmodule
