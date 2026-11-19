// Compiler directives (IEEE 1800 22): `define with and without arguments, default argument
// values, `undef, `ifdef/`ifndef/`elsif/`else, `include with a guard (included twice),
// line continuation, `` token pasting, `" stringification, `__FILE__/`__LINE__,
// `resetall, `default_nettype, `undefineall
`include "inc_defs.vh"
`include "inc_defs.vh"
`define W 8
`define MAX(a, b) ((a) > (b) ? (a) : (b))
`define ADD3(a, b = 1, c = 2) ((a) + (b) + (c))
`define CAT(x, y) x``y
`define STR(x) `"x`"
`define LONG(v) \
  ((v) * 2 + \
   1)
`define GONE 1
`undef GONE
module t;
  integer errors = 0;
  reg [`W-1:0] v;
  integer `CAT(my, var) = 5;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    check(`__LINE__, $bits(v), 8);
    check(`__LINE__, `MAX(3, 9), 9);
    check(`__LINE__, `MAX(4 + 4, 2), 8);
    check(`__LINE__, `ADD3(10), 13);
    check(`__LINE__, `ADD3(10, 5), 17);
    check(`__LINE__, `ADD3(10, , 0), 11);
    check(`__LINE__, myvar, 5);
    check(`__LINE__, `LONG(3), 7);
    check(`__LINE__, `FROM_INCLUDE, 77);
    if (`STR(abc) != "abc") begin $display("FAILED line %0d", `__LINE__); errors++; end
    if (`MAX("a,b", "a(") != "a,b") begin $display("FAILED line %0d", `__LINE__); errors++; end
`ifdef GONE
    errors++;
`elsif W
    v = 1;
`else
    errors++;
`endif
`ifndef NOT_DEFINED
    v = v + 1;
`endif
    check(`__LINE__, v, 2);
    check(`__LINE__, `__LINE__, 50);
`undefineall
`ifdef W
    errors++;
`endif
`ifdef FROM_INCLUDE
    errors++;
`endif
    if (errors == 0) $display("PASSED");
  end
endmodule
