// Methods of a package's queue or array called through its scoped name
// (`P::q.push_back(x)`, `P::q.size()`), also inside the package.
package pk;
  int q[$];
  int a[string];
  function void add(int x);
    pk::q.push_back(x);
  endfunction
endpackage

module top;
  initial begin
    pk::add(1);
    pk::q.push_back(2);
    pk::q.push_front(0);
    pk::a["x"] = 5;
    if (pk::q.size() == 3 && pk::q[0] == 0 && pk::q[2] == 2 && pk::a.num() == 1 && pk::a.exists("x"))
      $display("PASSED");
    else
      $display("FAILED: %p", pk::q);
  end
endmodule
