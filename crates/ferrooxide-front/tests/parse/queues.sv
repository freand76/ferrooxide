typedef int iq_t[$];
module top;
  int q[$], b[$:3];
  bit [3:0] d[];
  iq_t t;
  function automatic iq_t f(int a[$]);
    f = a[1:$];
  endfunction
  initial begin
    d = new[4];
    d = new[8](d);
    q = {};
    q = {q, 1};
    q[$] = q[$-1];
    q.push_back(2);
    t = f(q);
    foreach (q[i]) b.push_front(q[i]);
  end
endmodule
