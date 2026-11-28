// A variable declared `static` in a block is initialized once, before any process starts
// (IEEE 1800-2017 6.21): no change event at time 0, not again when the block is entered
// again; `automatic` ones are set at each entry. Also a class handle initializer.
module t;
  class C; int v = 5; endclass
  int n = 0, fails = 0;
  initial begin : sub
    static int i = 1;
    static int j = 0;
    #1;
    if (n != 0) begin $display("FAILED: initializer made an event"); fails++; end
    for (int k = 0; k < 3; k++) begin
      static int once = 10;
      automatic int each = 10;
      once++;
      each++;
      if (once != 11 + k || each != 11) begin $display("FAILED: %0d %0d at %0d", once, each, k); fails++; end
    end
    begin
      static C c = new;
      if (c == null || c.v != 5) begin $display("FAILED: handle"); fails++; end
    end
    if (fails == 0) $display("PASSED");
  end
  always @(sub.i) n++;
endmodule
