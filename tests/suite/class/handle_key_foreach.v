// foreach over an associative array property with class handle keys: the loop variable
// is a handle of the key's class (IEEE 1800-2017 12.7.3), whose members (an element
// written, methods of an array property) can be named.
class N;
  typedef bit edges_t[N];
  edges_t succ;
  int id;
  function new(int i);
    id = i;
  endfunction
  function void link(N o);
    succ[o] = 1;
  endfunction
  function int back_links(N w);
    int n = 0;
    foreach (w.succ[p]) begin
      p.succ[this] = 1;
      n += p.id;
    end
    return n;
  endfunction
  function void unlink(N w);
    foreach (w.succ[p])
      if (p.succ.exists(w)) p.succ.delete(w);
  endfunction
endclass

module t;
  integer errors = 0;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask

  N a, b, c;

  initial begin
    a = new(1);
    b = new(2);
    c = new(3);
    a.link(b);
    a.link(c);
    check(`__LINE__, a.back_links(a), 5);
    check(`__LINE__, b.succ.num(), 1);
    check(`__LINE__, c.succ.exists(a), 1);
    a.unlink(a);
    check(`__LINE__, b.succ.num(), 0);
    check(`__LINE__, c.succ.num(), 0);
    if (errors == 0) $display("PASSED");
  end
endmodule
