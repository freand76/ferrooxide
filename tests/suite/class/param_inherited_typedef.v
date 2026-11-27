// A typedef of a parameterized base class naming a specialization (`typedef R #(T)
// rsrc_t;`), used in a derived class: in each specialization of the derived class it is
// the base's type of the same parameter values (IEEE 1800-2017 8.25, 8.13).
class R #(type T = int);
  T v;
endclass
virtual class Impl #(type T = int);
  typedef R #(T) rsrc_t;
  pure virtual function int show(rsrc_t r);
endclass
class Def #(type T = int) extends Impl #(T);
  virtual function int show(rsrc_t r); return $bits(r.v); endfunction
  function int run();
    R #(T) r = new;
    return show(r);
  endfunction
endclass
module t;
  initial begin
    Def #(int) a = new;
    Def #(logic [99:0]) b = new;
    Def #(byte) c = new;
    if (a.run() == 32 && b.run() == 100 && c.run() == 8) $display("PASSED");
    else $display("FAILED: %0d %0d %0d", a.run(), b.run(), c.run());
  end
endmodule
