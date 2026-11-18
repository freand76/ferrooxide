// Classes (IEEE 1800-2017 8): properties of several types with initializers, handles
// (null, shared objects, comparison), constructors with arguments and default arguments,
// methods (functions and tasks) through `this`, a property hidden by a local, shallow copies.
class Point;
  int x, y = 2;
  logic [3:0] tag;
  string name = "p";
  real scale = 1.5;

  function new(int x0 = 7);
    x = x0;
  endfunction

  function int sum();
    return x + y;
  endfunction

  function void move(int dx, int dy = 1);
    this.x += dx;
    y = y + dy;
  endfunction

  task automatic shadow(output int r);
    int x = 100;  // hides the property
    r = x + this.x;
  endtask
endclass

class Box;
  Point corner;
  int id;
  function new(int i);
    id = i;
    corner = new(i * 10);
  endfunction
endclass

module m;
  Point p, q;
  Box b, c;
  int r;
  initial begin
    if (p != null) $display("FAILED -- handle starts as null");
    p = new;
    if (p.x != 7 || p.y != 2 || p.tag !== 4'bx || p.name != "p" || p.scale != 1.5)
      $display("FAILED -- start values %0d %0d %b %s %f", p.x, p.y, p.tag, p.name, p.scale);
    q = p;
    q.move(3);
    if (p.x != 10 || p.y != 3 || p.sum() != 13) $display("FAILED -- shared object %0d %0d", p.x, p.y);
    p.move(0, 2);
    q = new(1);
    if (q == p || q.x != 1) $display("FAILED -- new object");
    q.shadow(r);
    if (r != 101) $display("FAILED -- hidden property r=%0d", r);
    b = new(4);
    if (b.corner.x != 40 || b.id != 4) $display("FAILED -- nested object");
    c = new b;  // shallow: the corner is shared
    c.id = 5;
    c.corner.x = 41;
    if (b.id != 4 || b.corner.x != 41) $display("FAILED -- shallow copy %0d %0d", b.id, b.corner.x);
    $display("PASSED");
  end
endmodule
