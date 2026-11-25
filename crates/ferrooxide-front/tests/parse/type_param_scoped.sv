class B;
  static int k;
  static function int f(); return 1; endfunction
endclass
class U #(type T = B);
  static function int g();
    T::f();
    return T::k + T::f();
  endfunction
endclass
