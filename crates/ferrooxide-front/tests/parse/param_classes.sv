// parameterized classes: value and type parameter ports, specializations as types, as a
// base class (using the class's parameter), in `C #(..)::x` and `C #(..)::new`
class base_cls #(int b = 20);
  int a;
  static function int twice();
    return 2 * b;
  endfunction
endclass
class ext_cls #(int e = 25, type T = byte) extends base_cls #(e + 1);
  T v;
  function T get();
    return v;
  endfunction
endclass
module m;
  ext_cls #(15, int) inst;
  ext_cls d;
  initial begin
    inst = ext_cls#(.e(3))::new;
    $display(base_cls#()::b, base_cls#(4)::b + 1);
    base_cls#(6)::twice();
  end
endmodule
