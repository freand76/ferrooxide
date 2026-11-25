// expected
// shortreal variables, arguments and results hold a real (the reference keeps 64 bits).
module test;
  shortreal x, a[2];
  real y;
  function shortreal half(input shortreal v);
    half = v / 2;
  endfunction
  initial begin
    x = 0.1;
    y = x;
    a[1] = half(x);
    $display("%.20f %.20f %.20f", x, y, a[1]);
    $display("%g %0d", half(3.0), half(5) > 2.4);
  end
endmodule
