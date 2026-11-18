// Class inheritance (IEEE 1800-2017 8.13-8.17): base properties and methods, the base
// constructor run first (implicitly, through `super.new(..)` or with the arguments of
// `extends B(..)`), property initializers after it, `super.` members, a derived property
// hiding a base one; classes at file scope and in a package.
package shapes;
  class Shape;
    int sides;
    string kind = "shape";
    function new(int n);
      sides = n;
    endfunction
    function int corners();
      return sides;
    endfunction
  endclass
endpackage

class Counter;
  int n = 1;
  function void bump();
    n++;
  endfunction
endclass

class Twice extends Counter;
  int n = 10;  // hides Counter::n
  function void bump();
    super.bump();
    n += 2;
  endfunction
  function int base_n();
    return super.n;
  endfunction
endclass

module m;
  import shapes::*;

  class Square extends Shape;
    int side_len = sides * 2;  // after the base constructor
    function new(int len);
      super.new(4);
      side_len += len;
    endfunction
  endclass

  class Tri extends Shape(3);
    string kind = "tri";
  endclass

  Square s;
  Tri t;
  Twice w;
  initial begin
    s = new(5);
    if (s.sides != 4 || s.corners() != 4 || s.side_len != 13 || s.kind != "shape")
      $display("FAILED -- square %0d %0d %0d %s", s.sides, s.corners(), s.side_len, s.kind);
    t = new;
    if (t.sides != 3 || t.kind != "tri") $display("FAILED -- tri %0d %s", t.sides, t.kind);
    w = new;
    w.bump();
    if (w.n != 12 || w.base_n() != 2) $display("FAILED -- twice %0d %0d", w.n, w.base_n());
    $display("PASSED");
  end
endmodule
