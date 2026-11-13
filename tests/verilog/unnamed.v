// Instances without an instance name (UDPs; modules need a name)
primitive inv (o, i);
  output o; input i;
  table 0 : 1; 1 : 0; endtable
endprimitive
module pass (output o, input i);
  assign o = i;
endmodule
module top;
  reg a;
  wire b, c;
  inv (b, a);
  pass p1 (c, b);
  initial begin a = 0; #1 $display("%b %b", b, c); a = 1; #1 $display("%b %b", b, c); end
endmodule
