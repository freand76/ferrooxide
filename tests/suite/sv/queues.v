// Queues and dynamic arrays (IEEE 1800 7.5, 7.10): methods, element access, `$`, copies on
// assignment, concatenation, slices, `new[]`, out-of-range reads, function arguments and
// results, foreach.
typedef int iq_t[$];

module t;
  integer errors = 0;
  int q[$], r[$];
  logic [7:0] lq[$];
  real rq[$];
  string sq[$];
  bit [3:0] d[];
  int i, sum;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  function automatic int total(int a[$]);
    total = 0;
    foreach (a[k]) total += a[k];
  endfunction
  function automatic iq_t qrev(int n);
    for (int k = 0; k < n; k++) qrev.push_front(k);
  endfunction
  initial begin
    check(`__LINE__, q.size(), 0);
    q.push_back(2);
    q.push_back(3);
    q.push_front(1);
    check(`__LINE__, q.size(), 3);
    check(`__LINE__, q[0], 1);
    check(`__LINE__, q[$], 3);
    check(`__LINE__, q[$-1], 2);
    q.insert(1, 10);
    check(`__LINE__, q[1], 10);
    q.delete(1);
    check(`__LINE__, q[1], 2);
    // a copy: later changes of q leave r alone
    r = q;
    q[0] = 100;
    check(`__LINE__, r[0], 1);
    check(`__LINE__, q[0], 100);
    check(`__LINE__, q == r, 0);
    q[0] = 1;
    check(`__LINE__, q == r, 1);
    check(`__LINE__, q.pop_front(), 1);
    check(`__LINE__, q.pop_back(), 3);
    check(`__LINE__, q.size(), 1);
    q = {q, 7, 8};
    check(`__LINE__, q.size(), 3);
    check(`__LINE__, q[2], 8);
    q = {0, q};
    check(`__LINE__, q[0], 0);
    r = q[1:2];
    check(`__LINE__, r.size(), 2);
    check(`__LINE__, r[0], 2);
    check(`__LINE__, r[1], 7);
    q = '{5, 6};
    check(`__LINE__, total(q), 11);
    check(`__LINE__, total('{1, 2, 3}), 6);
    q = {};
    check(`__LINE__, q.size(), 0);
    r = qrev(3);
    check(`__LINE__, r[0], 2);
    check(`__LINE__, r[2], 0);
    // out of range: x for 4-state elements, 0 for 2-state ones
    check(`__LINE__, lq[0], 8'hxx);
    check(`__LINE__, q[5], 0);
    lq.push_back(8'hA5);
    check(`__LINE__, lq[0], 8'hA5);
    lq.delete();
    check(`__LINE__, lq.size(), 0);
    rq.push_back(1.5);
    rq.push_back(2.25);
    if (rq[0] + rq[1] != 3.75) begin
      $display("FAILED line %0d: real elements", `__LINE__);
      errors = errors + 1;
    end
    sq.push_back("ab");
    sq.push_back("cd");
    if ({sq[0], sq[$]} != "abcd") begin
      $display("FAILED line %0d: string elements", `__LINE__);
      errors = errors + 1;
    end
    // a dynamic array
    d = new[4];
    check(`__LINE__, d.size(), 4);
    check(`__LINE__, d[3], 0);
    foreach (d[k]) d[k] = k + 1;
    d = new[6](d);
    check(`__LINE__, d.size(), 6);
    check(`__LINE__, d[3], 4);
    check(`__LINE__, d[5], 0);
    sum = 0;
    for (i = 0; i < d.size; i++) sum += d[i];
    check(`__LINE__, sum, 10);
    d.delete();
    check(`__LINE__, d.size(), 0);
    if (errors == 0) $display("PASSED");
  end
endmodule
