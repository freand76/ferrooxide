// Typed constructor calls (IEEE 1800-2017 8.8): `C::new` makes an object of class C, which
// may be a class derived from the target's class or named through a typedef.
module test;
  class B;
    int x = 1;
    function int get(); return x; endfunction
  endclass
  class D extends B;
    int y;
    function new(int v = 5);
      x = 10 + v;
      y = v;
    endfunction
    function int get(); return x + y; endfunction
  endclass
  typedef D DT;

  int fails = 0;
  B b;
  D d;
  initial begin
    b = B::new;
    if (b.x != 1) begin $display("FAILED B::new %0d", b.x); fails++; end
    b = D::new(2);
    if (b.x != 12 || b.get() != 12) begin $display("FAILED D::new(2) %0d", b.x); fails++; end
    d = DT::new;
    if (d.x != 15 || d.y != 5) begin $display("FAILED DT::new %0d %0d", d.x, d.y); fails++; end
    d = D::new();
    if (d.get() != 20) begin $display("FAILED D::new() %0d", d.get()); fails++; end
    if (fails == 0) $display("PASSED");
  end
endmodule
