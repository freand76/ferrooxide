// expected
// A run-time warning of randomize() names the line of the call, also when an inlined
// function call comes before it in the same expression.
module t;
  class C;
    rand bit [3:0] x;
    constraint c { x > 4'd14; x < 4'd3; }
  endclass
  function automatic int one(int a);
    int b;
    b = a + 1;
    return b - a;
  endfunction
  C c;
  int ok;
  initial begin
    c = new;
    ok = one(5) +
         c.randomize();
    $display("ok=%0d", ok);
  end
endmodule
