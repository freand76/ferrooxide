// flags: --elab-only
// expected
// --elab-only parses, elaborates and compiles the design and stops: nothing runs, so
// nothing is printed (constant functions still run, as part of elaboration).
module m;
  function integer f(input integer x);
    f = x * 2;
  endfunction
  localparam P = f(21);
  reg [P-1:0] r;
  initial $display("FAILED -- this ran (%0d)", $bits(r));
  final $display("FAILED -- final ran");
endmodule
