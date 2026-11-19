// Forward typedefs of classes (IEEE 1800-2017 6.18, 8.27): `typedef class C;` and
// `typedef C;` before and after the class, handles of a class declared later, also as
// properties of another class.
module t;
  integer errors = 0;

  typedef class C;
  typedef C;
  typedef class B;

  C x;

  class A;
    B b;
    function new();
      b = new;
    endfunction
  endclass

  class C;
    int v = 4;
  endclass

  class B;
    int w = 7;
  endclass

  typedef class C;

  C y;
  A a;

  initial begin
    x = new;
    y = x;
    a = new;
    if (y.v !== 4 || a.b.w !== 7 || $bits(x.v) != 32) begin
      $display("FAILED");
      errors = errors + 1;
    end
    if (errors == 0) $display("PASSED");
  end
endmodule
