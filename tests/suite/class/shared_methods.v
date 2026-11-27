// Methods called from several processes, recursively and through each other: each call
// has its own automatic variables (IEEE 1800-2017 8.6, 13.4.2), arguments, results and
// output arguments; a static function's variables are shared by all calls.
class Node;
  Node kids[$];
  int val;
  function new(int v);
    val = v;
  endfunction
  function void add(Node n);
    kids.push_back(n);
  endfunction
  // a recursive method with a local queue and a foreach over it
  function int sum();
    Node c[$];
    int s = val;
    foreach (kids[i]) c.push_back(kids[i]);
    foreach (c[i]) s += c[i].sum();
    return s;
  endfunction
  // recursion through two methods, with output arguments
  function void depth(output int d, input int base = 0);
    int best = base;
    foreach (kids[i]) begin
      int k;
      kids[i].depth_of(k, base + 1);
      if (k > best) best = k;
    end
    d = best;
  endfunction
  function void depth_of(output int d, input int base);
    depth(d, base);
  endfunction
endclass
class Counter;
  static int total;
  int mine;
  function int bump(int by);
    int prev = mine;
    mine += by;
    total += by;
    return prev;
  endfunction
endclass
module t;
  int bad = 0;
  int done = 0;
  Node root;
  Counter cs[2];
  // a static function: one set of variables for every caller
  function int seq();
    int last;
    last++;
    return last;
  endfunction
  function automatic int fact(int n);
    if (n <= 1) return 1;
    return n * fact(n - 1);
  endfunction
  task automatic worker(int id);
    int r, d;
    for (int i = 0; i < 5; i++) begin
      r = root.sum();
      if (r != 21) bad++;
      root.depth(d);
      if (d != 3) bad++;
      if (cs[id].bump(id + 1) != i * (id + 1)) bad++;
      if (fact(id + 4) != (id == 0 ? 24 : 120)) bad++;
      #1;
    end
    done++;
  endtask
  initial begin
    Node a, b, c, d, e;
    root = new(1);
    a = new(2); b = new(3); c = new(4); d = new(5); e = new(6);
    root.add(a); root.add(b); a.add(c); c.add(d); b.add(e);
    cs[0] = new; cs[1] = new;
    fork
      worker(0);
      worker(1);
    join
    if (Counter::total != 15 || cs[0].mine != 5 || cs[1].mine != 10) bad++;
    if (seq() != 1 || seq() != 2) bad++;
    if (done != 2) bad++;
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %0d", bad);
  end
endmodule
