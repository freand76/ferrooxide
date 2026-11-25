// Extern methods with their bodies outside the class (IEEE 1800-2017 8.24): the body sees the
// class's properties and types, default argument values come from the prototype; also a
// constructor, a task, a static and a virtual method, and a class in a package.
package p;
  class K;
    int v;
    extern function new(int v0 = 4);
    extern function int get();
  endclass
  function K::new(int v0 = 4);
    v = v0;
  endfunction
  function int K::get();
    return v;
  endfunction
endpackage
module t;
  class C;
    typedef logic [7:0] byte_t;
    int a;
    static int n;
    extern function byte_t twice(int x = 3);
    extern task bump(int by);
    extern static function int count();
    extern virtual function string name();
  endclass
  function C::byte_t C::twice(int x);
    byte_t r = x * 2;
    return r + a;
  endfunction
  task C::bump(int by);
    #1 a += by;
    n++;
  endtask
  function int C::count();
    return n;
  endfunction
  function string C::name();
    return "C";
  endfunction
  class D extends C;
    extern virtual function string name();
  endclass
  function string D::name();
    return "D";
  endfunction
  initial begin
    C c = new;
    D d = new;
    p::K k = new;
    int bad = 0;
    c.a = 1;
    if (c.twice() != 7 || c.twice(10) != 21) bad++;
    c.bump(5);
    d.bump(1);
    if (c.a != 6 || C::count() != 2) bad++;
    c = d;
    if (c.name() != "D") bad++;
    if (k.get() != 4) bad++;
    if (bad) $display("FAILED"); else $display("PASSED");
  end
endmodule
