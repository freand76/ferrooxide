// A parameterized class extends a specialization of its base built from
// its own parameters (`extends B #(string, T)`, IEEE 1800-2017 8.25):
// each specialization of the class has its own base.
module top;
  class pool #(type KEY = int, T = int);
    T items[KEY];
    function void add(KEY k, T v); items[k] = v; endfunction
    function T get(KEY k); return items[k]; endfunction
  endclass

  class spool #(type T = byte) extends pool #(string, T);
    function int count(); return items.num(); endfunction
  endclass

  initial begin
    spool #(int) p = new;
    spool #(byte) q = new;
    p.add("a", 1000);
    p.add("b", 2);
    q.add("c", 8'hff);
    if (p.get("a") == 1000 && p.count() == 2 && q.get("c") == -1 && q.count() == 1) $display("PASSED");
    else $display("FAILED: %0d %0d %0d %0d", p.get("a"), p.count(), q.get("c"), q.count());
  end
endmodule
