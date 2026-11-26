// More of what class libraries use (IEEE 1800-2017 8.25, 8.10, 26.7, 7.8): nested
// associative arrays with class handle keys and struct elements as a property (written
// with an assignment pattern), a property whose type is a specialization named at file
// scope (declared after the classes), a class of package std (`process`) as a property
// type in a package, a class of another package as a parameter of `mailbox`, a method
// call on a static property in an out-of-body static method, a static property of a
// parameterized class initialized by a static method that calls a specialization's
// static method, a method of a handle whose type is a type parameter, and a struct
// property reached through a queue of handles.
package lib;
  class Item;
    int v;
  endclass
  class Holder;
    process p;
    mailbox #(Item) mb = new;
  endclass
endpackage

class K;
endclass

typedef struct { int st; bit rv; } si_t;
typedef struct { int a; string n; } pair_t;

class Pool #(type KEY = int, type T = int);
  T items[KEY];
endclass

typedef Pool #(string, int) Pool_t;

class C;
  si_t r[K][K][int];
  Pool_t pools[string];
  local static bit names[string];
  extern static function bit define(string name);
endclass

function bit C::define(string name);
  if (names.exists(name)) return 0;
  names[name] = 1;
  return 1;
endfunction

typedef class Counter;
typedef class Item0;

class Reg #(type T = int);
  static function bit reg_it();
    return Counter #(T)::bump();
  endfunction
  static bit done = reg_it();
endclass

class Counter #(type T = int);
  static int n;
  static function bit bump();
    n++;
    return 1;
  endfunction
endclass

class Wrap #(type T = Item0);
  static function int use_it();
    T h = new;
    return h.get();
  endfunction
endclass

class Item0;
  function int get();
    return 42;
  endfunction
endclass

class Ovr;
  pair_t orig;
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
    C c = new;
    K a = new, b = new;
    lib::Holder h = new;
    lib::Item it = new, got;
    Ovr q[$];
    Reg #(byte) rg;
    c.r[a][b][1] = '{3, 1};
    check(`__LINE__, c.r[a][b][1].st == 3 && c.r[a][b][1].rv == 1);
    check(`__LINE__, c.r.exists(a) && !c.r.exists(b) && c.r[a].num() == 1);
    c.pools["x"] = new;
    c.pools["x"].items["k"] = 5;
    check(`__LINE__, c.pools["x"].items["k"] == 5);
    h.p = process::self();
    check(`__LINE__, h.p.status() == process::RUNNING);
    it.v = 9;
    h.mb.put(it);
    h.mb.get(got);
    check(`__LINE__, got.v == 9);
    check(`__LINE__, C::define("a") && !C::define("a"));
    check(`__LINE__, Reg #(byte)::done == 1 && Counter #(byte)::n == 1);
    check(`__LINE__, Wrap #(Item0)::use_it() == 42);
    q.push_back(new);
    q[0].orig.a = 4;
    q[0].orig.n = "o";
    check(`__LINE__, q[0].orig.a == 4 && q[0].orig.n == "o");
    if (errors == 0) $display("PASSED");
  end
endmodule
