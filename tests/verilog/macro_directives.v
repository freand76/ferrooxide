// Directives in macro text: continued lines end in newlines in the expansion (IEEE 1800
// 22.5.1), so `undef/`ifdef/`define/`endif produced by a macro work, also through another
// macro; line numbers after multi-line expansions stay right.
`define DEFIF_A_OR_NOT_B(A,B,C) \
    `undef C \
    `ifdef A \
        `define C \
    `endif \
    `ifndef B \
        `define C \
    `endif
`define DEFIF_NOT_A_OR_B(A,B,C) `DEFIF_A_OR_NOT_B(B,A,C)
`define HAVE_B
`DEFIF_NOT_A_OR_B(HAVE_A, HAVE_B, R1)
`DEFIF_A_OR_NOT_B(HAVE_A, HAVE_B, R2)
module m;
  initial begin
`ifdef R1
    $display("R1 defined");
`endif
`ifndef R2
    $display("R2 not defined");
`endif
    $display("line %0d", `__LINE__);
  end
endmodule
