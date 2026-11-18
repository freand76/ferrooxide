// `void'(f(...))` calls a function and drops its value (IEEE 1800-2017 13.4.1): user
// functions of any type, methods of built-in types, system functions with side effects.
module test;
  int a, q[$];
  real b;
  string c;
  integer fd, ch;

  function int f1(int x);
    a = x;
    return x;
  endfunction
  function real f2(real x);
    b = x;
    return x;
  endfunction
  function string f3(string x);
    c = x;
    return x;
  endfunction

  initial begin
    void'(f1(10));
    void'(f2(1.5));
    void'(f3("10"));
    q.push_back(1);
    q.push_back(2);
    void'(q.pop_front());
    void'($clog2(10));
    if (a === 10 && b == 1.5 && c == "10" && q.size() == 1 && q[0] == 2)
      $display("PASSED");
    else
      $display("FAILED");
  end
endmodule
