// Queues, dynamic and associative arrays of class handles (IEEE 1800 7.5, 7.8, 7.10, 8.4):
// elements made with `new`, members and methods of elements and of popped handles, null
// for missing elements, objects shared between arrays, arrays of handles as properties.
class Item;
  int v;
  function new(int x);
    v = x;
  endfunction
  function int twice();
    return 2 * v;
  endfunction
endclass

class Sub extends Item;
  function new(int x);
    super.new(x + 100);
  endfunction
endclass

class Box;
  Item items[$];
  function void put(Item i);
    items.push_back(i);
  endfunction
endclass

module t;
  integer errors = 0;
  Item q[$], d[], a[string], it;
  Sub s;
  Box b;
  int n;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    q.push_back(new(1));
    q.push_back(new(2));
    s = new(3);
    q.push_front(s);
    check(`__LINE__, q.size(), 3);
    check(`__LINE__, q[0].v, 103);
    check(`__LINE__, q[2].twice(), 4);
    it = q.pop_back();
    check(`__LINE__, it.v, 2);
    check(`__LINE__, q.pop_front().v, 103);
    check(`__LINE__, q[5] == null, 1);
    n = 0;
    foreach (q[k]) n += q[k].v;
    check(`__LINE__, n, 1);
    // the same object in two arrays
    d = new[2];
    d[1] = q[0];
    d[1].v = 9;
    check(`__LINE__, q[0].v, 9);
    check(`__LINE__, d[0] == null, 1);
    a["x"] = it;
    check(`__LINE__, a["x"].v, 2);
    check(`__LINE__, a["y"] == null, 1);
    // a property holding handles
    b = new;
    b.put(it);
    b.put(new(5));
    check(`__LINE__, b.items.size(), 2);
    check(`__LINE__, b.items[1].v, 5);
    check(`__LINE__, b.items[0] == it, 1);
    if (errors == 0) $display("PASSED");
  end
endmodule
