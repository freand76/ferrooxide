// Programs (IEEE 1800-2017 24): a top-level program, a program instantiated by a module
// with ports, and a program and a module declared inside a module (23.4: without ports
// they are instantiated once under their own name, and see the enclosing module's names).
program top_prog;
  int n;
  initial for (n = 0; n < 4; n++) ;
  final if (n !== 4) $display("FAILED -- top program n=%0d", n);
endprogram

program checker_prog(input [7:0] a, input [7:0] b);
  final begin
    if (a !== 8'h55 || b !== 8'haa) $display("FAILED -- ports a=%h b=%h", a, b);
    else $display("PASSED");
  end
endprogram

module main;
  timeunit 1ns;
  timeprecision 1ps;
  reg [7:0] shared;
  wire [7:0] inv = ~shared;
  checker_prog chk(shared, inv);

  program setter;
    initial shared <= 8'h55;
  endprogram

  module inner;
    localparam W = 3;
    reg [W-1:0] seen;
    initial #2 seen = shared[W-1:0];
  endmodule

  initial begin
    #5;
    if (inner.seen !== 3'b101) $display("FAILED -- inner.seen=%b", inner.seen);
    if ($bits(inner.seen) != 3) $display("FAILED -- width");
    $printtimescale(inner);
  end
endmodule
