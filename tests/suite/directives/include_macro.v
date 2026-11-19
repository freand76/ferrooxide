// `include with the file name given by a macro (IEEE 1800-2017 22.4: the file name may be
// a text macro that expands to a quoted name), also a stringified one.
`define INC "inc_defs.vh"
`define INC2(base) `"base.vh`"
`include `INC
`include `INC2(inc_defs) // a comment after it
module m;
  initial begin
    if (`FROM_INCLUDE != 77) $display("FAILED -- %0d", `FROM_INCLUDE);
    $display("PASSED");
  end
endmodule
