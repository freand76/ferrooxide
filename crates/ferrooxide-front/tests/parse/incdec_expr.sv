module m;
  integer i, j;
  reg [3:0] x;
  initial begin
    j = i++ + --i;
    x[i++] = 1;
    if (j && i++) j = 0;
  end
endmodule
