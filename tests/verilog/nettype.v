// `default_nettype for implicit nets; pullups on single bits of a vector
`default_nettype wand
module top;
  reg a, b;
  assign w = a;
  assign w = b;
  wire [2:0] out;
  reg sel;
  pullup (out[0]);
  pulldown (out[2]);
  assign out = sel ? 3'b010 : 3'bzzz;
  initial begin
    a = 0; b = 1; sel = 0;
    #1 $display("%b %b", w, out);
    a = 1; sel = 1;
    #1 $display("%b %b", w, out);
  end
endmodule
`default_nettype wire
