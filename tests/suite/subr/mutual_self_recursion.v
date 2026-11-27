// Automatic functions that call themselves and each other (IEEE 1800-2017 13.4.2): each
// call has its own variables.
module t;
  function automatic int f(int n);
    int k = n;
    if (n <= 0) return 1;
    return f(n - 1) + g(n - 1) + k - n;
  endfunction
  function automatic int g(int n);
    int k = n * 2;
    if (n <= 0) return 2;
    return g(n - 1) + f(n - 2) + k - 2 * n;
  endfunction
  // f(n) = f(n-1) + g(n-1), g(n) = g(n-1) + f(n-2), with f(<=0) = 1, g(<=0) = 2
  initial begin
    if (f(1) == 3 && g(1) == 3 && f(2) == 6 && g(2) == 4 && f(5) == 30 && g(5) == 23) $display("PASSED");
    else $display("FAILED: %0d %0d %0d %0d %0d %0d", f(1), g(1), f(2), g(2), f(5), g(5));
  end
endmodule
