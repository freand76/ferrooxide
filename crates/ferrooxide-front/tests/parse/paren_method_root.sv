class A;
  int v;
  function int f(); return v; endfunction
endclass
module t;
  A a, b;
  int i, x;
  initial begin
    x = (i ? a : b).f();
    x = (i ? a : b).v;
    x = (a).f();
  end
endmodule
