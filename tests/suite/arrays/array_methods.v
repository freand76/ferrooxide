// Array manipulation methods of queues and dynamic arrays (IEEE 1800-2017 7.12):
// reduction, locator and ordering methods, with and without `with`.
class C;
  int v;
  function new(int x);
    v = x;
  endfunction
endclass

module test;
  typedef enum { R, B, G } E;
  E es[] = '{B, R, G};
  C cs[$];
  int d[] = '{5, -3, 8, 1, 8};
  byte b[] = '{1, 2, 3, 4};
  string s[$] = '{"pear", "apple", "fig", "apple"};
  int q[$], r[$];
  string sq[$];
  int fails = 0;

  task check(input bit ok, input string what);
    if (!ok) begin
      $display("FAILED: %s", what);
      fails++;
    end
  endtask

  initial begin
    // reductions
    check(d.sum == 19, "sum");
    check(b.sum == 10 && b.product == 24 && b.and == 0 && b.or == 7 && b.xor == 4, "byte reductions");
    check(d.sum() with (item > 2 ? item : 0) == 21, "sum with");
    check(d.sum with (int'(item.index)) == 10, "sum of indexes");
    check(d.product(x) with (x == 8 ? 2 : 1) == 4, "product with a named iterator");
    // locators
    q = d.find with (item > 4);
    check(q.size == 3 && q[0] == 5 && q[1] == 8 && q[2] == 8, "find");
    q = d.find_index with (item == 8);
    check(q.size == 2 && q[0] == 2 && q[1] == 4, "find_index");
    q = d.find_first with (item < 2);
    check(q.size == 1 && q[0] == -3, "find_first");
    q = d.find_last_index with (item < 2);
    check(q.size == 1 && q[0] == 3, "find_last_index");
    q = d.find_first with (item > 100);
    check(q.size == 0, "find_first without a match");
    q = d.min;
    check(q.size == 1 && q[0] == -3, "min (signed)");
    q = d.max;
    check(q.size == 1 && q[0] == 8, "max");
    q = d.max with (-item);
    check(q.size == 1 && q[0] == -3, "max with");
    q = d.unique;
    check(q.size == 4 && q[0] == 5 && q[1] == -3 && q[2] == 8 && q[3] == 1, "unique");
    q = d.unique_index;
    check(q.size == 4 && q[3] == 3, "unique_index");
    q = d.unique with (item % 2);
    check(q.size == 3 && q[0] == 5 && q[1] == -3 && q[2] == 8, "unique with");
    sq = s.find with (item == "apple");
    check(sq.size == 2 && sq[0] == "apple", "find of strings");
    sq = s.min;
    check(sq.size == 1 && sq[0] == "apple", "min of strings");
    sq = s.unique;
    check(sq.size == 3, "unique strings");
    // `with` inside `with`: the inner iterator is its own
    r = '{1, 2};
    q = d.find with (r.sum with (item * 2) == 6 && item == 1);
    check(q.size == 1 && q[0] == 1, "nested with");
    // ordering
    d.sort;
    check(d[0] == -3 && d[1] == 1 && d[2] == 5 && d[3] == 8 && d[4] == 8, "sort (signed)");
    d.rsort;
    check(d[0] == 8 && d[4] == -3, "rsort");
    d.reverse;
    check(d[0] == -3 && d[4] == 8, "reverse");
    s.sort;
    check(s[0] == "apple" && s[1] == "apple" && s[2] == "fig" && s[3] == "pear", "sort of strings");
    d.shuffle;
    check(d.sum == 19 && d.size == 5, "shuffle keeps the elements");
    // ordering by a `with` key (stable for equal keys)
    d = '{3, 1, 2, 5};
    d.sort with (-item);
    check(d[0] == 5 && d[1] == 3 && d[2] == 2 && d[3] == 1, "sort with");
    d.rsort(x) with (x % 3);
    check(d[0] == 5 && d[1] == 2 && d[2] == 1 && d[3] == 3, "rsort with");
    s.sort(x) with (x.len());
    check(s[0] == "fig" && s[3] == "apple", "sort of strings with a string method");
    // the iterator has the element's type: enum and class members
    q = es.find_index with (item.name == "G");
    check(q.size == 1 && q[0] == 2, "enum method of the iterator");
    cs.push_back(new(5));
    cs.push_back(new(7));
    cs.push_back(new(1));
    check(cs.sum with (item.v) == 13, "class property of the iterator");
    cs.sort with (item.v);
    check(cs[0].v == 1 && cs[1].v == 5 && cs[2].v == 7, "sort of handles with a property");
    if (fails == 0) $display("PASSED");
  end
endmodule
