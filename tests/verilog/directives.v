// iverilog: -g2005
// Directives with comments around arguments, comments in macros, #1 'h.. delays,
// @hier.event, `unconnected_drive
`define show(a, b) \
  $display("%0d", a /* sum \
  of both */ + b); // done \
  $display("macro continues")

/* c */ `default_nettype /* c */ // c
/* c */ wire /* c */ // c
`unconnected_drive pull1
module sub (input i, output o);
  assign o = i;
endmodule
`nounconnected_drive
module top;
  reg [31:0] dat;
  event ev;
  wire o;
  sub s ();
  sub s2 (.i(), .o(o));
  initial begin
    `show(2, 3);
    dat = #1 'h00010203;
    $display("%h %b %b", dat, s.i, o);
    #1 ->ev;
  end
  initial begin
    @top.ev $display("event at %0t", $time);
  end
endmodule
