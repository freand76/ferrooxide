// Function calls in continuous assignments and @* are sensitive to the arguments only (Icarus)
module top;
  reg g, a;
  reg o;
  wire w = f(a);
  function f; input x; f = x ^ g; endfunction
  always @* o = f(a);
  initial begin
    a = 0; g = 0;
    #1 $display("%b %b", o, w);
    g = 1;
    #1 $display("%b %b", o, w);
    a = 1;
    #1 $display("%b %b", o, w);
  end
endmodule
