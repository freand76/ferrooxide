// Recursive functions and tasks (automatic), also in constant expressions
module top;
  function automatic [15:0] factorial(input [15:0] n);
    factorial = (n > 1) ? factorial(n - 1) * n : n;
  endfunction
  function automatic integer fib(input integer n);
    if (n < 2) fib = n;
    else fib = fib(n - 1) + fib(n - 2);
  endfunction
  function integer cfact(input integer n);   // static, used as a constant function
    if (n > 1) cfact = n * cfact(n - 1); else cfact = 1;
  endfunction
  task automatic count(input integer n, output integer s);
    integer t;
    if (n == 0) s = 0;
    else begin
      count(n - 1, t);
      s = t + n;
    end
  endtask
  localparam P = cfact(6);
  integer r;
  wire [15:0] w = factorial(5);
  initial begin
    $display("%0d %0d %0d", factorial(3), factorial(7), fib(10));
    count(10, r);
    #1 $display("%0d %0d %0d", r, P, w);
  end
endmodule
