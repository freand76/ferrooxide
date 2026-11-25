// Static methods and properties of a class used before its body, after `typedef class`
// (IEEE 1800-2017 8.27): as calls, as a statement and as an assignment target.
typedef class S;
class U;
  static function int f();
    S s;
    s = S::get();
    S::k = 4;
    S::bump(3);
    return s.v + S::k;
  endfunction
endclass
class S;
  int v = 5;
  static int k;
  static function S get(); S r = new; return r; endfunction
  static function void bump(int n); k += n; endfunction
endclass
module t;
  initial if (U::f() == 12) $display("PASSED"); else $display("FAILED %0d", U::f());
endmodule
