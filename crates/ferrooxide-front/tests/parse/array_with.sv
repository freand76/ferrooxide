// array methods with `with`: the iterator (default and named), its index, nesting,
// an ordering method as a statement
module m;
  int d[$], q[$], r[$];
  string s[];
  initial begin
    q = d.find with (item > 2);
    q = d.find_index(x) with (x == x.index);
    $display(d.sum with (int'(item.index)));
    q = d.find with (r.sum(y) with (y * item) > 0);
    d.sort with (-item);
    s.rsort(e) with (e.len());
    d.reverse;
  end
endmodule
