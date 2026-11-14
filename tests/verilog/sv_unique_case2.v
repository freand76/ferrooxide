// unique/priority case warnings: time in ticks of the precision, scope of the instance
`timescale 1ns/1ps
module sub(input [1:0] s, output reg [3:0] y);
  always @* begin
    priority case (s)
      2'd0: y = 1;
      2'd1: y = 2;
      2'd2: y = 4;
    endcase
  end
endmodule
module t;
  reg [1:0] s; wire [3:0] y; reg [3:0] z;
  sub u(s, y);
  genvar g;
  for (g = 0; g < 1; g = g + 1) begin : gb
    always @(s) unique casex (s) 2'b0x: z = 1; 2'b11: z = 3; endcase
  end
  initial begin
    $timeformat(-9, 2, " ns", 8);
    #0.5 s = 0; #1.5 s = 2; #1 s = 1; #1 s = 3;
    #1 unique case (s) 2'd3: unique case (s) 2'd0: z = 0; endcase endcase
    #1 $display("%0d %0d", y, z);
  end
endmodule
