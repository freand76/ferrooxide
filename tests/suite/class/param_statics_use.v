// Parameterized classes: `C::f` of a static function without arguments is a call
// (IEEE 1800-2017 13.4.5); inside a specialization the class's own name in `C::m()` (and
// a call of a static method without `C::`) is that specialization (8.25.1); static
// properties exist for the specializations in use only (8.25), so an initializer of the
// default specialization that would be invalid is not compiled while that one is unused;
// a named argument names a static property.
class R #(type T = int);
  static int n = T::count();
endclass
class M;
  static function int count();
    return 7;
  endfunction
endclass
class K #(int W = 4);
  static int w = f();
  static function int f();
    return W * 2;
  endfunction
  static function string type_name();
    return $sformatf("K%0d", W);
  endfunction
endclass
class U #(type T = int);
  static function string nm();
    return {"<", T::type_name, ">"};
  endfunction
endclass
class C #(type T = int);
  typedef C#(T) this_type;
  int id;
  static function this_type get();
    this_type x = new;
    x.id = $bits(T);
    return x;
  endfunction
  static function int via_own_name();
    this_type inst = C::get();
    return inst.id;
  endfunction
  static function int via_call();
    this_type inst = get();
    return inst.id;
  endfunction
endclass
class P;
  int v;
  function new(int x);
    v = x;
  endfunction
  function int find(int k);
    return v + k;
  endfunction
  function int add(int a, int b = 0);
    return a + b;
  endfunction
endclass
class D;
  static P m[string];
  static function int get();
    m["common"] = new(5);
    return m["common"].add(1, .b(m["common"].find(2)));
  endfunction
endclass

module t;
  integer errors = 0;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask

  initial begin
    check(`__LINE__, R#(M)::n, 7);
    check(`__LINE__, K::w, 8);
    check(`__LINE__, K#(5)::w, 10);
    if (U#(K#(3))::nm() != "<K3>") begin
      $display("FAILED line %0d: %s", `__LINE__, U#(K#(3))::nm());
      errors = errors + 1;
    end
    if (K#(6)::type_name != "K6") begin
      $display("FAILED line %0d", `__LINE__);
      errors = errors + 1;
    end
    check(`__LINE__, C#(byte)::via_own_name(), 8);
    check(`__LINE__, C#(shortint)::via_call(), 16);
    check(`__LINE__, D::get(), 8);
    if (errors == 0) $display("PASSED");
  end
endmodule
