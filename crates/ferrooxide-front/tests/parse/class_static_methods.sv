class C;
  static int count = 0;
  static function int twice(int x);
    return 2 * x;
  endfunction
  static task bump(int n);
    count += n;
  endtask
  function int both(int v);
    bump(1);
    return twice(v);
  endfunction
endclass
module m;
  C c;
  int r;
  initial begin
    r = C::twice(4);
    C::bump(2);
    r = c.twice(r) + C::count;
  end
endmodule
