// A package's own types named with the package scope inside the package (IEEE 1800-2017 26.3).
package p;
  typedef class C;
  typedef logic [3:0] nib_t;
  class D;
    function int f();
      p::C c;
      p::nib_t n = 4'hf;
      c = new;
      return c.v + n;
    endfunction
  endclass
  class C;
    int v = 7;
  endclass
endpackage
module t;
  initial begin
    p::D d = new;
    if (d.f() == 22) $display("PASSED"); else $display("FAILED %0d", d.f());
  end
endmodule
