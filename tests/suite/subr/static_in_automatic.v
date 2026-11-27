// A variable declared `static` in an automatic subroutine is one variable for all calls
// (IEEE 1800-2017 6.21), initialized once: a counter, a singleton of a class's static
// method, and a recursive function whose calls all count in the same variable. Each
// specialization of a parameterized class has its own (also a block's `static` variable).
class P #(int N = 1);
  static function P #(N) get();
    static P #(N) inst;
    if (inst == null) inst = new();
    return inst;
  endfunction
  static function int bump();
    begin
      static int k = N;
      k++;
      return k;
    end
  endfunction
endclass
class S;
  static function S get();
    static S inst;
    if (inst == null) inst = new();
    return inst;
  endfunction
endclass
module t;
  function automatic int count();
    static int n = 10;
    int a = 0;
    n++;
    a++;
    return n * 10 + a;
  endfunction
  function automatic int depth(int k);
    static int calls = 0;
    calls++;
    if (k > 0) void'(depth(k - 1));
    return calls;
  endfunction
  initial begin
    int bad = 0;
    P #(2) dummy;
    if (count() != 111 || count() != 121 || count() != 131) bad++;
    if (S::get() != S::get() || S::get() == null) bad++;
    if (P #(1)::get() != P #(1)::get() || P #(2)::get() != P #(2)::get()) bad++;
    if ($cast(dummy, P #(1)::get()) && dummy == P #(2)::get()) bad++;
    if (P #(1)::bump() != 2 || P #(2)::bump() != 3 || P #(1)::bump() != 3 || P #(2)::bump() != 4) bad++;
    if (depth(3) != 4 || depth(0) != 5) bad++;
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %0d", bad);
  end
endmodule
