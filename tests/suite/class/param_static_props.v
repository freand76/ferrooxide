// Each specialization of a parameterized class has its own static
// properties (IEEE 1800-2017 8.25.1): read and written by its methods and
// as `C #(..)::x`; equal parameter values give the same specialization.
module top;
  typedef enum {A, B, C} abc_e;

  class wrap #(type T = int);
    static int count = 0;
    protected static T map[string];
    static function void add(string n, T v);
      map[n] = v;
      count++;
    endfunction
    static function bit get(string n, ref T v);
      if (!map.exists(n)) return 0;
      v = map[n];
      return 1;
    endfunction
  endclass

  int fails = 0;
  task check(input bit ok, input string what);
    if (!ok) begin
      $display("FAILED: %s", what);
      fails++;
    end
  endtask

  initial begin
    abc_e e;
    int i;
    wrap#(abc_e)::add("b", B);
    wrap#(abc_e)::add("c", C);
    wrap#(int)::add("x", 42);
    wrap#(byte)::add("y", 1);
    check(wrap#(abc_e)::count == 2, "count of wrap#(abc_e)");
    check(wrap#(int)::count == 1 && wrap#()::count == 1, "count of wrap#(int), the default");
    check(wrap#(abc_e)::get("c", e) && e == C, "an enum element");
    check(wrap#(int)::get("x", i) && i == 42, "an int element");
    check(!wrap#(int)::get("b", i), "a missing element");
    wrap#(int)::count = 10;
    wrap#(int)::count += 1;
    check(wrap#(int)::count == 11 && wrap#(byte)::count == 1, "written from outside");
    if (fails == 0) $display("PASSED");
  end
endmodule
