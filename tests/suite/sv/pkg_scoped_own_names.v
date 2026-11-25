// A package's own variables, functions and enum constants named with its scope inside the
// package (IEEE 1800-2017 26.3), read and assigned.
package p;
  typedef enum {A, B} e_t;
  int v = 4;
  function int f(); return p::v + 1; endfunction
  function int g(); p::v = 6; return p::f() + p::B; endfunction
endpackage
module t;
  initial begin
    int a = p::f();
    int b = p::g();
    if (a == 5 && b == 8) $display("PASSED"); else $display("FAILED %0d %0d", a, b);
  end
endmodule
