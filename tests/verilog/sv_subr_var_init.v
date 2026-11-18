// Initial values of variables declared in functions and tasks (IEEE 1800-2017 6.21): a static
// variable gets it once, an automatic one on every call; a constant function call starts
// afresh and does not change the run-time variables. `module automatic` makes subroutines
// automatic unless they say otherwise.
module test;
  function integer acc_s(input integer v);
    static integer acc = 10;
    acc = acc + v;
    return acc;
  endfunction
  function automatic integer acc_a(input integer v);
    automatic integer acc = 10;
    acc = acc + v;
    return acc;
  endfunction
  task count;
    static integer n = 100;
    real r = 0.5;
    begin
      n = n + 1;
      r = r * 2;
      $display("count n=%0d r=%0.1f", n, r);
    end
  endtask
  localparam P1 = acc_s(1), P2 = acc_s(2), P3 = acc_a(3);
  initial begin
    $display("P %0d %0d %0d", P1, P2, P3);
    $display("static %0d %0d", acc_s(1), acc_s(2));
    $display("auto %0d %0d", acc_a(1), acc_a(2));
    count;
    count;
  end
endmodule

module automatic m2;
  function integer f(input integer v);
    integer acc = 1;
    acc = acc + v;
    return acc;
  endfunction
  function static integer g(input integer v);
    static integer acc = 1;
    acc = acc + v;
    return acc;
  endfunction
  initial begin
    #1 $display("m2 auto %0d %0d", f(1), f(2));
    $display("m2 static %0d %0d", g(1), g(2));
  end
endmodule
