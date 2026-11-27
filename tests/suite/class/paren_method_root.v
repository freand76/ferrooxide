// A member or method of a parenthesized primary (method_call_root, IEEE 1800-2017
// A.8.2): `(c ? a : b).f()` calls f of the handle the conditional gives.
class A;
  int v;
  function new(int x); v = x; endfunction
  function int f(); return v * 10; endfunction
endclass
module t;
  A a = new(1), b = new(2);
  int bad = 0;
  initial begin
    for (int i = 0; i < 2; i++) begin
      if ((i ? a : b).f() != (i ? 10 : 20)) bad++;
      if ((i ? a : b).v != (i ? 1 : 2)) bad++;
    end
    if ((a).f() != 10) bad++;
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %0d", bad);
  end
endmodule
