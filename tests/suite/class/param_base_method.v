// A class that extends a type parameter (IEEE 1800-2017 8.25) calls methods through a
// handle property its base declares, without `this.`: `m.name()`.
class N;
  string s;
  function new(string x); s = x; endfunction
  function string name(); return s; endfunction
endclass
class B;
  protected N m = new("b");
  int k = 5;
endclass
class C;
  N m = new("c");
  int k = 7;
endclass
class S #(type BASE = B) extends BASE;
  function string f(); return {m.name(), "."}; endfunction
  function int g(); return k; endfunction
endclass
class D extends S #(C);
  function string h(); return m.name(); endfunction
endclass
module t;
  S s = new;
  S #(C) sc = new;
  D d = new;
  int bad = 0;
  initial begin
    if (s.f() != "b." || s.g() != 5) bad++;
    if (sc.f() != "c." || sc.g() != 7) bad++;
    if (d.h() != "c") bad++;
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %0d", bad);
  end
endmodule
