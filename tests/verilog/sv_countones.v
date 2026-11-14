module t;
  reg [7:0] a; reg [99:0] b; integer i;
  initial begin
    a = 8'b0010_1100; $display("%0d %b %b", $countones(a), $onehot(a), $onehot0(a));
    a = 8'b0000_0100; $display("%0d %b %b", $countones(a), $onehot(a), $onehot0(a));
    a = 8'b0; $display("%0d %b %b", $countones(a), $onehot(a), $onehot0(a));
    a = 8'b01x0_z000; $display("%0d %b %b", $countones(a), $onehot(a), $onehot0(a));
    a = 8'b00x0_z000; $display("%0d %b %b", $countones(a), $onehot(a), $onehot0(a));
    a = 8'b11x0_z000; $display("%0d %b %b", $countones(a), $onehot(a), $onehot0(a));
    b = 0; b[99] = 1; b[3] = 1; $display("%0d %b %b", $countones(b), $onehot(b), $onehot0(b));
    b = 0; b[70] = 1; $display("%0d %b %b", $countones(b), $onehot(b), $onehot0(b));
    i = $countones(a) + 1; $display("%0d %0d", i, $countones(a[1:0]+3'd7));
    $display("%0d", $countones(4'b1111) == 4);
    $display("%b", $onehot(3'b100) ? 2'b10 : 2'b01);
    $display("%h", $countones(a));
  end
endmodule
