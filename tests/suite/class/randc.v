// randc (IEEE 1800-2017 18.4.2): a randc variable goes through a random permutation of
// its values; no value repeats before every value has been given. Constraints narrow the
// values of a cycle; each object, array element and static property has its own cycle.
class A;
  randc bit [2:0] x;
  randc bit [3:0] y;
  randc bit [1:0] e[2];
  rand bit [7:0] r;
  constraint c { y inside {[3:7]}; r < 10; }
endclass
class C;
  randc bit [3:0] z;
  constraint c { z inside {1, 5, 9}; }
endclass
class B;
  static randc bit [2:0] s;
endclass
module t;
  A a = new, b = new, c = new;
  B p = new, q = new;
  C k = new;
  int bad = 0;
  bit [7:0] seen;
  bit [15:0] seen_y;
  initial begin
    for (int cyc = 0; cyc < 3; cyc++) begin
      seen = 0;
      for (int i = 0; i < 8; i++) begin
        if (!a.randomize()) bad++;
        if (seen[a.x]) bad++;
        seen[a.x] = 1;
        if (a.r >= 10) bad++;
      end
      if (seen != 8'hff) bad++;
    end
    // a cycle of the five values the constraint leaves
    for (int cyc = 0; cyc < 3; cyc++) begin
      seen_y = 0;
      for (int i = 0; i < 5; i++) begin
        if (!b.randomize()) bad++;
        if (b.y < 3 || b.y > 7 || seen_y[b.y]) bad++;
        seen_y[b.y] = 1;
      end
    end
    // array elements, a static property (shared by the objects), randomize(v)
    begin
      bit [3:0] s0 = 0, s1 = 0;
      seen = 0;
      for (int i = 0; i < 4; i++) begin
        if (!a.randomize()) bad++;
        if (s0[a.e[0]] || s1[a.e[1]]) bad++;
        s0[a.e[0]] = 1;
        s1[a.e[1]] = 1;
      end
      if (s0 != 4'hf || s1 != 4'hf) bad++;
      for (int i = 0; i < 8; i++) begin
        B o = i % 2 ? p : q;
        if (!o.randomize()) bad++;
        if (seen[B::s]) bad++;
        seen[B::s] = 1;
      end
      seen = 0;
      c.y = 4;
      for (int i = 0; i < 8; i++) begin
        if (!c.randomize(x)) bad++;
        if (seen[c.x] || c.r != 0 || c.y != 4) bad++;
        seen[c.x] = 1;
      end
    end
    // a set of single values; a fixed value repeats (each cycle holds one value)
    for (int cyc = 0; cyc < 4; cyc++) begin
      seen_y = 0;
      for (int i = 0; i < 3; i++) begin
        if (!k.randomize()) bad++;
        if (!(k.z inside {1, 5, 9}) || seen_y[k.z]) bad++;
        seen_y[k.z] = 1;
      end
    end
    repeat (3) if (!k.randomize() with { z == 5; } || k.z != 5) bad++;
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %0d", bad);
  end
endmodule
