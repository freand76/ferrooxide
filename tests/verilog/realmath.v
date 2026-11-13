// Real math functions
module top;
  real r;
  integer i;
  initial begin
    r = 2.0; i = 9;
    $display("%f %f %f %f", $sqrt(r), $sqrt(i), $ln(r), $log10(1000.0));
    $display("%f %f %f %f", $exp(1.0), $floor(-2.5), $ceil(2.1), $pow(2, 10));
    $display("%f %f %f", $sin(0.5), $atan2(1.0, 2.0), $hypot(3, 4));
    $display("%f %f %f", $cos(r), $tan(0.3), $acos(0.5));
  end
endmodule
