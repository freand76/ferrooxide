// programs, a program with ports, nested program and module declarations, pkg::param
package pk;
  localparam int W = 4;
endpackage

program top_prog;
  int n;
  initial n = pk::W;
  final $display(n);
endprogram

program chk(input [7:0] a);
  final $display(a);
endprogram

module outer;
  timeunit 1ns;
  reg [7:0] s;
  chk c(s);
  program setter;
    initial s <= 8'h55;
  endprogram
  module inner;
    timeunit 10ns;
    reg r;
    initial r = s[0];
  endmodule
endmodule
