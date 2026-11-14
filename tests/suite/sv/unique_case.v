// expected
// unique / priority case: a value no item matches is reported (IEEE 1800 12.5.3); unique0
// and cases with a default are silent
`timescale 1ns/1ps
module t;
  reg [1:0] s;
  reg [3:0] y;
  always @(s) begin
    unique case (s)
      2'd0: y = 1;
      2'd1: y = 2;
    endcase
    priority casez (s)
      2'b1?: y = 3;
    endcase
    unique0 case (s)
      2'd3: y = 4;
    endcase
    unique case (s)
      2'd0: y = 5;
      default: y = 6;
    endcase
  end
  initial begin
    #0.5 s = 0;
    #1 s = 2;
    #1 s = 1;
    #1 $display("done y=%0d", y);
  end
endmodule
