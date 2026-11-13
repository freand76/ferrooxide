// Real-valued nets (wire real)
module top;
  real rin, scale;
  wire real ress = scale * rin;
  wire real sum;
  assign sum = ress + 0.5;
  initial begin
    rin = 1.5; scale = 2.0;
    #1 $display("%f %f", ress, sum);
    rin = -3.25;
    #1 $display("%f %f", ress, sum);
  end
endmodule
