// Arguments bound by name in calls of functions, tasks and methods
module m;
  function automatic int f(int a, int b = 1);
    return a + b;
  endfunction
  task t(input int x, output int y);
    y = x;
  endtask
  initial begin
    int r;
    r = f(.b(2), .a(1));
    r = f(3, .b());
    t(.y(r), .x(f(.a(r))));
  end
endmodule
