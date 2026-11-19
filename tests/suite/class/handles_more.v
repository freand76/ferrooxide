// Class handles returned by functions (IEEE 1800-2017 8.4: a handle is a value like any
// other), a base class named through its package (`extends P::B`, 8.13) while the module
// declares a class of the same name, and `this.super.x` (8.15).
package P;
  class B;
    int v = 1;
    function int who();
      return 100;
    endfunction
  endclass
endpackage

module m;
  class B;
    int v = 2;
    function int who();
      return 200;
    endfunction
  endclass

  class D extends P::B;
    byte v;  // hides P::B::v
    function void set(int x);
      this.super.v = x;
      v = x + 1;
    endfunction
    function int base_v();
      return this.super.v;
    endfunction
  endclass

  function D make(int x);
    D d;
    d = new;
    d.set(x);
    return d;
  endfunction

  B b;
  D d;
  initial begin
    b = new;
    d = make(7);
    if (d.who() != 100 || b.who() != 200) $display("FAILED -- who %0d %0d", d.who(), b.who());
    if (d.base_v() != 7 || d.v != 8) $display("FAILED -- v %0d %0d", d.base_v(), d.v);
    if (make(3).v != 4) $display("FAILED -- make(3).v");
    $display("PASSED");
  end
endmodule
