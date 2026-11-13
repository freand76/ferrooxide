// Drive strengths: the strongest driver wins, equal strengths that disagree give x
module top;
  reg d, oe_n;
  reg [1:0] pv;
  wire out, w2, w3, w4;
  wire [1:0] bus;
  supply0 gnd;
  bufif0 (out, d, oe_n);
  assign (pull0, pull1) out = pv[0];
  assign (weak0, weak1) w2 = pv[1];
  assign (pull0, pull1) w2 = d;
  assign (strong0, highz1) w3 = d;       // open drain
  assign (pull1, highz0) w3 = 1'b1;      // pullup resistor
  assign (weak0, weak1) bus = pv;
  assign bus = {d, 1'bz};
  assign w4 = gnd;
  assign (supply1, supply0) w4 = d;
  initial begin
    pv = 2'b10; oe_n = 0; d = 0;
    #1 $display("%b %b %b %b %b", out, w2, w3, bus, w4);
    d = 1;
    #1 $display("%b %b %b %b %b", out, w2, w3, bus, w4);
    oe_n = 1;
    #1 $display("%b %b %b %b %b", out, w2, w3, bus, w4);
    d = 1'bx;
    #1 $display("%b %b %b %b %b", out, w2, w3, bus, w4);
  end
endmodule
