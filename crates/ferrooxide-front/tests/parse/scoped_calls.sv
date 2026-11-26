// statements that start with a scoped name: static method calls through a package's
// parameterized class or a class's typedef, not declarations; a member as a delay
package P;
  class D #(type T = int);
    typedef int I;
    static function void set(T a); endfunction
  endclass
endpackage
class R;
  static function void bump(int a); endfunction
endclass
class E;
  typedef R reg_t;
  typedef int J;
endclass
class Cfg;
  int offset = 7;
endclass
module m;
  E::J q;
  Cfg cfg;
  initial begin
    P::D #(int)::set(1);
    E::reg_t::bump(5);
    #cfg.offset;
  end
endmodule
