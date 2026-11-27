// rand dynamic arrays (IEEE 1800-2017 18.4, 18.5.8.1): a constraint on size() decides the
// new size; without one the size stays. The elements are random in both cases.
// `with` constrains the sizes too, also of queues (a bounded one up to its bound). The objects of a rand array of handles are randomized (null elements are skipped).
class Item;
  rand bit [3:0] v;
  constraint k { v > 4'd9; }
endclass
class Pkt;
  rand byte unsigned data[];
  rand bit [99:0] wide[];
  rand int unsigned len;
  rand Item items[];
  constraint c_size { data.size() == len; len inside {[2:6]}; wide.size() < 3; wide.size() > 0; }
endclass
class Fixed;
  rand bit [7:0] q[$];
endclass
class Fifo;
  int m_size = 4;
  rand bit [15:0] fifo[$];
  rand bit [3:0] bnd[$:2];
  constraint k { fifo.size() <= m_size; fifo.size() >= 2; bnd.size() > 0; }
endclass
module t;
  Pkt p = new;
  Fixed f = new;
  Fifo ff = new;
  int bad = 0;
  int changed = 0;
  initial begin
    p.items = new[3];
    p.items[0] = new;
    p.items[2] = new;
    repeat (20) begin
      if (!p.randomize()) bad++;
      if (p.data.size() != p.len || p.len < 2 || p.len > 6) bad++;
      if (p.wide.size() < 1 || p.wide.size() > 2) bad++;
      if (p.items.size() != 3 || p.items[1] != null) bad++;
      if (p.items[0].v <= 9 || p.items[2].v <= 9) bad++;
      foreach (p.data[i]) if (p.data[i] != p.data[0]) changed++;
    end
    if (!p.randomize() with { data.size() == 5; wide.size() == 1; }) bad++;
    if (p.data.size() != 5 || p.len != 5 || p.wide.size() != 1) bad++;
    f.q = '{0, 0, 0, 0, 0, 0, 0, 0};
    if (!f.randomize()) bad++;
    if (f.q.size() != 8) bad++;
    foreach (f.q[i]) if (f.q[i] != 0) changed++;
    ff.fifo = '{1, 2, 3, 4, 5, 6, 7};
    repeat (10) begin
      if (!ff.randomize()) bad++;
      if (ff.fifo.size() < 2 || ff.fifo.size() > 4 || ff.bnd.size() < 1 || ff.bnd.size() > 3) bad++;
    end
    if (bad == 0 && changed > 0) $display("PASSED");
    else $display("FAILED: %0d %0d", bad, changed);
  end
endmodule
