// Static properties of a parameterized base class are shared with the
// classes derived from its specialization; a class refers to a
// specialization of itself (`cbs #(T, CB)::get()`) and to static
// properties through a typedef of a specialization; a static method's
// result takes method calls (`C::get().m()`, IEEE 1800-2017 8.25, 8.10).
module top;
  class typed #(type T = int);
    static int count;
    static typed #(T) inst;
    string name;
    static function typed #(T) get();
      if (inst == null) inst = new;
      count++;
      return inst;
    endfunction
  endclass

  class cbs #(type T = int, type CB = byte) extends typed #(T);
    typedef typed #(T) base_t;
    static function int self_use();
      void'(cbs #(T, CB)::get());
      void'(cbs #(int, int)::get());
      base_t::inst.name = "set";
      return cbs #(T, CB)::count + count;
    endfunction
  endclass

  class root;
    static root inst;
    int v = 7;
    static function root get();
      if (inst == null) inst = new;
      return inst;
    endfunction
    function int twice(); return 2 * v; endfunction
  endclass

  initial begin
    int fails = 0;
    if (cbs #(byte, int)::self_use() != 2) fails++;
    if (typed #(byte)::count != 1 || typed #(int)::count != 1) fails++;
    if (typed #(byte)::inst.name != "set") fails++;
    if (root::get().twice() != 14 || root::get().v != 7) fails++;
    root::inst.v = 1;
    if (root::get().v != 1) fails++;
    if (fails == 0) $display("PASSED");
    else $display("FAILED: %0d", fails);
  end
endmodule
