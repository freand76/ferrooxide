// Parameterized classes as used by class libraries (IEEE 1800-2017 8.25, 8.24, 6.18): the
// type parameters of a base class in a derived class, a type parameter as the base class
// (its members named without `this.`), `C::T` in an out-of-body method, a typedef after
// the class named in such a method, a static method through a typedef of a type
// parameter's class (`T::id::f()`), a method of a static property through the class
// (`C::q.delete()`), a specialization of a class derived from the class that uses it, and
// a string parameter passed on.
class Item;
  int v = 5;
  typedef Item self_t;
  static function string who();
    return "item";
  endfunction
endclass

virtual class Proxy #(type T = int, int N = 3);
  pure virtual function void get(T s, ref T out[$]);
endclass

class IProxy extends Proxy #(Item, 4);
  virtual function void get(T s, ref T out[$]);
    T x = s;
    out.push_back(x);
  endfunction
  function int n();
    return N;
  endfunction
endclass

class Iface;
  int base_v = 1;
  function int tag();
    return 7;
  endfunction
endclass

class Port #(type IF = Iface) extends IF;
  function int twice();
    return 2 * tag() + base_v;
  endfunction
endclass

class Who #(type T = Item);
  static function string name();
    return T::self_t::who();
  endfunction
endclass

class Store #(type K = int);
  extern function K first(K d);
  extern function int count();
endclass

typedef Store #(int) int_store;

function Store::K Store::first(Store::K d);
  int_store s = new;
  return d + s.count();
endfunction

function int Store::count();
  return 1;
endfunction

class Reg;
  static int all[$];
  static function void clear();
    Reg::all.delete();
  endfunction
endclass

// (A uses a specialization of a class derived from A)
typedef class D;
class A;
  function int f();
    D #(3) d = new;
    return d.k;
  endfunction
endclass

class D #(int K = 1) extends A;
  int k = K;
endclass

class Named #(type T = int, string S = "<none>");
  typedef Named #(T, S) this_type;
  static function string name();
    return S;
  endfunction
endclass

module t;
  integer errors = 0;
  task check(input integer line, input bit ok);
    if (!ok) begin
      $display("FAILED line %0d", line);
      errors = errors + 1;
    end
  endtask
  initial begin
    IProxy p = new;
    Item i = new, q[$];
    Port #(Iface) pt = new;
    Store #(int) st = new;
    A a = new;
    p.get(i, q);
    check(`__LINE__, q.size() == 1 && q[0].v == 5 && p.n() == 4);
    check(`__LINE__, pt.twice() == 15);
    check(`__LINE__, Who#(Item)::name() == "item");
    check(`__LINE__, st.first(4) == 5);
    Reg::all.push_back(1);
    Reg::clear();
    check(`__LINE__, Reg::all.size() == 0);
    check(`__LINE__, a.f() == 3);
    check(`__LINE__, Named#(int, "n")::this_type::name() == "n");
    if (errors == 0) $display("PASSED");
  end
endmodule
