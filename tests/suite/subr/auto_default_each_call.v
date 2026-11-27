// An automatic variable without an initializer starts at its type's default value on
// each call, and one of a block on each entry of the block (IEEE 1800-2017 6.21): a queue
// starts empty, a 4-state vector x, an int 0, a string empty, a handle null; a static
// variable keeps its value between calls.
class C;
  int v;
endclass
module t;
  int bad = 0;
  function automatic int f(int a);
    int q[$];
    logic [3:0] x;
    int n;
    string s;
    C h;
    static int calls;
    calls++;
    if (q.size() != 0 || x !== 4'bxxxx || n != 0 || s != "" || h != null) bad++;
    q.push_back(a);
    x = 4'b0101;
    n = a;
    s = "set";
    h = new;
    for (int i = 0; i < 3; i++) begin
      int k;
      logic [1:0] y;
      int bq[$];
      if (k != 0 || y !== 2'bxx || bq.size() != 0) bad++;
      k = i + 1;
      y = 2'b10;
      bq.push_back(i);
    end
    return calls;
  endfunction
  class K;
    function int g();
      int q[$];
      bit [7:0] b;
      if (q.size() != 0 || b != 0) bad++;
      q.push_back(1);
      b = 8'hff;
      return q.size();
    endfunction
  endclass
  initial begin
    K k = new;
    repeat (3) void'(f(7));
    if (f(1) != 4) bad++;
    repeat (3) if (k.g() != 1) bad++;
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %0d", bad);
  end
endmodule
