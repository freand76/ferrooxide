// randomize() reaches objects of the same class through rand handles (IEEE 1800-2017
// 18.4): a list through a rand handle, a tree through a rand array of handles of its own
// class, and a derived class held by a handle of its base. Every object gets values that
// satisfy its constraints; the rand_mode of nothing changes, null handles end the walk.
class Node;
  rand bit [3:0] v;
  rand Node next;
  constraint k { v inside {[3:5]}; }
endclass
class Tree;
  rand bit [7:0] x;
  rand Tree kids[];
  constraint k { x > 8'd200; }
endclass
class Leaf extends Tree;
  rand bit [7:0] y;
  constraint ky { y == x - 8'd100; }
endclass
module t;
  Node head = new;
  Tree root = new;
  Leaf lf = new;
  int bad = 0;
  function automatic int check(Tree t);
    Leaf l;
    int n = 1;
    if (t.x <= 200) bad++;
    if ($cast(l, t) && l.y != l.x - 100) bad++;
    foreach (t.kids[i]) if (t.kids[i] != null) n += check(t.kids[i]);
    return n;
  endfunction
  initial begin
    head.next = new;
    head.next.next = new;
    root.kids = new[3];
    root.kids[0] = new;
    root.kids[2] = lf;
    lf.kids = new[1];
    lf.kids[0] = new;
    repeat (10) begin
      if (!head.randomize()) bad++;
      if (!(head.v inside {[3:5]}) || !(head.next.v inside {[3:5]}) || !(head.next.next.v inside {[3:5]})) bad++;
      if (!root.randomize()) bad++;
      if (check(root) != 4) bad++;
    end
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %0d", bad);
  end
endmodule
