// Releasing a net that nothing drives (only forces wrote it) leaves it floating: z, bit by
// bit for a partial release.
module top;
  wire [1:0] out;
  wire [3:0] w;
  reg [1:0] in1, in2;
  initial begin
    $monitor($time,, "out=%d w=%b", out, w);
    in1 = 2'd1;
    in2 = 2'd2;
    #1 force out = in1;
    force w = 4'b1010;
    #1 force out = in2;
    #1 release out;
    release w[1:0];
    #1 release w;
  end
endmodule

// a driven net, partly released: the released bits follow the driver, the others stay forced
module driven;
  reg [3:0] r = 4'b0000;
  wire [3:0] d = r;
  wire [3:0] e;
  assign e = ~r;
  initial begin
    #10 force d = 4'b1111; force e = 4'b0000;
    #1 release d[1:0]; release e[3];
    #1 $display("%0t d=%b e=%b", $time, d, e);
    r = 4'b0101;
    #1 $display("%0t d=%b e=%b", $time, d, e);
    release d; release e;
    #1 $display("%0t d=%b e=%b", $time, d, e);
  end
endmodule
