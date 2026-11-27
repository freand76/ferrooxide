// An array output of a method gets a new array on each call (IEEE 1800-2017 6.21,
// 13.5): calls from several processes at once, whose results are kept while they wait,
// each see only their own elements.
module top;
  class P;
    typedef bit edges_t[P];
    P succ[$];
    string name;
    function new(string n); name = n; endfunction
    function void preds(output edges_t e);
      foreach (succ[i]) e[succ[i]] = 1;
    endfunction
    task check(int d, output int n);
      edges_t sib;
      preds(sib);
      sib[this] = 1;
      #d;
      n = sib.num();
    endtask
  endclass
  P a, b, c, x;
  int na, nb;
  initial begin
    a = new("a"); b = new("b"); c = new("c"); x = new("x");
    a.succ.push_back(c);
    b.succ.push_back(x);
    b.succ.push_back(c);
    fork
      a.check(2, na);
      b.check(1, nb);
    join
    $display("%0d %0d", na, nb);
    if (na == 2 && nb == 3) $display("PASSED"); else $display("FAILED");
  end
endmodule
