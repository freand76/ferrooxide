// Static methods called as statements through a class's typedef of another class
// (`C::T::m(..)`, as UVM's `type_id::create`) and through a specialization of a package's
// class (`P::C #(..)::m(..)`), and a delay given by a member (`#cfg.offset`).
package P;
  class D #(type T = int);
    static T n;
    static function void set(T a); n += a; endfunction
  endclass
endpackage
class R;
  static int k;
  static function void bump(int a); k += a; endfunction
endclass
class E;
  typedef R reg_t;
endclass
class Cfg;
  int offset = 7;
endclass
module m;
  Cfg cfg = new;
  initial begin
    E::reg_t::bump(5);
    E::reg_t::bump(2);
    P::D #(byte)::set(3);
    P::D #(byte)::set(4);
    #cfg.offset;
    if (R::k == 7 && $time == 7 && P::D #(byte)::n == 7 && $bits(P::D #(byte)::n) == 8)
      $display("PASSED");
    else $display("FAILED: %0d %0t", R::k, $time);
  end
endmodule
