// casts to real (IEEE 1800-2017 6.24.1): an integral value by its own signedness, a real
// unchanged
module top;
  typedef real r_t;
  parameter real P = real'(7) / 2;
  int i = 5;
  logic [3:0] l = 4'b1010;
  logic signed [3:0] sl = 4'b1010;
  real x = 2.75;
  int fails = 0;
  initial begin
    if (real'(i) != 5.0) fails++;
    if (real'(l) != 10.0) fails++;
    if (real'(sl) != -6.0) fails++;
    if (real'(-7) != -7.0) fails++;
    if (real'(x) != 2.75) fails++;
    if (r_t'(i) / 2 != 2.5) fails++;
    if (real'(i) / real'(2) != 2.5) fails++;
    if (P != 3.5) fails++;
    if (int'(real'(i) / 2) != 3) fails++;
    if (fails == 0) $display("PASSED");
    else $display("FAILED %0d", fails);
  end
endmodule
