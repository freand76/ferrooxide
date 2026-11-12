// Real numbers: variables, arrays, parameters (with overrides), arithmetic, comparisons,
// conversions, real task arguments and function results, %f/%e/%g formatting.
module m #(parameter real P = 1.5, parameter Q = 2.25, parameter integer K = 3) ();
  initial #1 $display("m: P=%f Q=%g K=%0d P*K=%f", P, Q, K, P * K);
endmodule

module tb;
  m m0();
  m #(.P(0.125), .Q(7), .K(2.6)) m1();
  real a, b, arr [0:2];
  real neg = -1.0e9;
  integer i;
  reg [7:0] r8;
  function real half(input real x);
    half = x / 2.0;
  endfunction
  task show(input real v, input integer k);
    $display("show %f %0d %e", v, k, v * k);
  endtask
  initial begin
    a = 3.25; b = -0.5;
    $display("%f %f %f %f %f", a + b, a - b, a * b, a / b, a ** 2.0);
    $display("%b %b %b %b %b %b", a > b, a < b, a == 3.25, a != b, a >= 3.25, b <= -1.0);
    i = a; r8 = 2.5; $display("i=%0d r8=%0d %0d %0d", i, r8, $rtoi(2.7), $rtoi(-2.7));
    i = -7; a = i; b = 3; $display("%f %f %f", a, b, a / 2);
    a = 1.0 / 3.0;
    $display("[%f] [%.2f] [%0.1f] [%10.3f] [%e] [%.3e] [%g] [%g] [%g]", a, a, a, a, a, a, a, 1.0e-6, 123456789.0);
    $display("%g %g %g %g", 0.0001, 100000.0, 1000000.0, 0.5);
    arr[0] = 1.5; arr[1] = half(arr[0]); arr[2] = arr[0] + arr[1];
    for (i = 0; i < 3; i = i + 1) $display("arr[%0d] = %f", i, arr[i]);
    show(2.5, 4);
    show(i, 3);
    $display("neg=%e %f", neg, neg + 1.0e9);
    $display("%0d %0d", 7.5 > 7, 3 + 0.5);
    $display("%f", $itor(5) / 2);
    a = 2.0; b = a * 1.0e3 + 0.75;
    $display("%f %0.1f %d", b, b, b);
  end
endmodule
