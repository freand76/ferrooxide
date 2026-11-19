// Static methods (IEEE 1800-2017 8.10): called as `C::m(..)`, through a handle (even a
// null one), from the class's own methods and through a derived class; they use static
// properties; static tasks as statements.
class C;
  static int count = 0;
  int id;
  function new();
    count++;
    id = next_id();
  endfunction
  static function int next_id();
    return count * 10;
  endfunction
  static function int twice(int x);
    return 2 * x;
  endfunction
  static task bump(int n);
    count += n;
  endtask
  function int both();
    return twice(id);
  endfunction
endclass

class D extends C;
  static function int thrice(int x);
    return twice(x) + x;
  endfunction
endclass

module t;
  integer errors = 0;
  C a, n;
  D d;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    check(`__LINE__, C::twice(4), 8);
    a = new;
    check(`__LINE__, a.id, 10);
    check(`__LINE__, C::count, 1);
    check(`__LINE__, a.twice(5), 10);
    check(`__LINE__, n.twice(6), 12);
    C::bump(3);
    check(`__LINE__, C::count, 4);
    check(`__LINE__, D::thrice(2), 6);
    check(`__LINE__, D::twice(7), 14);
    d = new;
    check(`__LINE__, d.id, 50);
    check(`__LINE__, a.both(), 20);
    check(`__LINE__, C::next_id, 50);
    if (errors == 0) $display("PASSED");
  end
endmodule
