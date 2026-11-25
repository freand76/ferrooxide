class R #(type T = int);
  static function void f(int a); endfunction
endclass
virtual class C;
  typedef logic [7:0] byte_t;
  int m[3];
  extern function byte_t g(int x = 3);
  extern task t();
  pure virtual function int h();
endclass
function C::byte_t C::g(int x);
  byte_t r = x;
  return r;
endfunction
task C::t();
endtask
