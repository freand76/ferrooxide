// Implicit nets (assign targets, gate terminals),
// specparams and $bits of a type
module top;
  wire out;
  assign in = 1'b0;
  assign out = ~in;
  assign {c1, c2} = 2'b10;
  not g1 (n1, c1);
  and g2 (n2, n1, c2);
  integer i;
  specify
    specparam Tdelay = 1.5;
  endspecify
  specparam Tw = 3;
  initial begin
    #1 $display("%b %b %b %b %b %b", out, in, c1, c2, n1, n2);
    $display("%0d %0d %0d %0d", $bits(integer), $bits(time), $bits(i), Tw);
    $display("%f", Tdelay);
  end
endmodule
