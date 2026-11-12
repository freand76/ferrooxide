// Combinational chains that get fused into one process: an assign feeding an always @* that
// must not run at time 0 (its input never changes, so its output stays x), and a longer chain.
module tb;
  reg [7:0] in;
  reg en;
  wire [7:0] s1 = in + 1;
  wire [7:0] s2 = s1 ^ 8'h55;
  reg [7:0] y, z;
  always @* begin
    y = 0;
    if (en) y = s2;
  end
  wire never = en & 1'b1;
  always @* begin
    z = 8'h11;
    if (never) z = 8'h22;
  end
  initial begin
    #1 $display("y=%h z=%h s2=%h", y, z, s2);
    in = 3;
    #1 $display("y=%h z=%h s2=%h", y, z, s2);
    en = 1;
    #1 $display("y=%h z=%h s2=%h", y, z, s2);
    in = 8'hf0; en = 0;
    #1 $display("y=%h z=%h s2=%h", y, z, s2);
  end
endmodule
