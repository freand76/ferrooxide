// Static class properties (IEEE 1800-2017 8.9): one variable shared by all objects of the
// class and its subclasses, reached as `p` in methods, `this.p`, `obj.p` (also through a
// null handle) and `C::p`.
package pkg;
  class Counter;
    static int count = 100;
    int id;
    function new();
      count++;
      id = count;
    endfunction
  endclass
endpackage

class Base;
  static int created;
  static string tag = "base";
  function new();
    created = created + 1;
  endfunction
  function int get();
    return this.created;
  endfunction
endclass

class Derived extends Base;
  function new();
    super.new();
    created += 10;
  endfunction
endclass

module test;
  import pkg::*;
  Base b1, b2, none;
  Derived d;
  Counter c1, c2;
  integer fails = 0;

  task check(input integer got, input integer want, input integer line);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d, want %0d", line, got, want);
      fails++;
    end
  endtask

  initial begin
    check(none.created, 0, `__LINE__);
    b1 = new;
    b2 = new;
    check(Base::created, 2, `__LINE__);
    d = new;
    check(b1.created, 13, `__LINE__);
    check(d.get(), 13, `__LINE__);
    Base::created = 5;
    check(d.created, 5, `__LINE__);
    if (Derived::tag != "base") begin $display("FAILED tag"); fails++; end
    c1 = new;
    c2 = new;
    check(c1.id, 101, `__LINE__);
    check(c2.id, 102, `__LINE__);
    check(Counter::count, 102, `__LINE__);
    if (fails == 0) $display("PASSED");
  end
endmodule
