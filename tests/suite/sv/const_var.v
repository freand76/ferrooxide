// const variables (IEEE 1800-2017 6.20.6): written only by their initializers; the
// object of a const class handle can still change
module top;
  class C;
    int a;
    task add(int v);
      a += v;
    endtask
  endclass
  const C obj = new;
  const int k = 5;
  const int j = k * 2;
  function automatic int f(int x);
    const int d = x + 1;
    return d * 2;
  endfunction
  initial begin
    const int b = 3;
    obj.add(4);
    obj.add(1);
    if (obj.a == 5 && k == 5 && j == 10 && b == 3 && f(10) == 22 && f(1) == 4)
      $display("PASSED");
    else
      $display("FAILED %0d %0d %0d %0d %0d", obj.a, k, j, b, f(10));
  end
endmodule
