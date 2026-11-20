// randomize() (IEEE 1800-2017 18): rand properties, constraint blocks (ranges, inside,
// dist, equalities between random variables, implication, if-else, foreach, unique),
// inheritance and overriding of constraints, randomize() with, pre/post_randomize,
// std::randomize, an unsatisfiable call.
module test;
  class B;
    rand int x;
    rand bit [3:0] y;
    rand int z;
    rand byte arr[4];
    int lim = 10;
    int posts = 0;
    constraint c1 { x > 0; x < lim; }
    constraint c2 { y inside {1, 3, [8:9]}; }
    constraint c3 { z == x + 100; }
    constraint c4 { foreach (arr[i]) arr[i] == i * 2; }
    function void post_randomize();
      posts++;
    endfunction
  endclass

  class D extends B;
    rand int w;
    rand int u, v;
    rand bit mode;
    constraint c1 { x == 42; }  // replaces B's c1
    constraint cw { w dist {5 := 1, [10:11] :/ 2, 7 := 0}; }
    constraint cm { mode -> u == 3; if (!mode) u inside {[20:22]}; else v == u * 2; }
    constraint cu { unique {u, w}; }
  endclass

  B b;
  D d;
  int ok, a, fails = 0;
  int seen_w[int];

  task check(input bit c, input string what);
    if (!c) begin
      $display("FAILED: %s", what);
      fails++;
    end
  endtask

  initial begin
    b = new;
    d = new;
    repeat (50) begin
      ok = b.randomize();
      check(ok == 1 && b.x > 0 && b.x < 10 && b.z == b.x + 100, "ranges and an equality");
      check(b.y inside {1, 3, 8, 9}, "inside");
      check(b.arr[0] == 0 && b.arr[1] == 2 && b.arr[3] == 6, "foreach");
    end
    check(b.posts == 50, "post_randomize");
    // the object's class decides (randomize() is virtual)
    b = d;
    repeat (50) begin
      ok = b.randomize();
      check(ok == 1 && d.x == 42 && d.z == 142, "derived constraint replaces the base one");
      check(d.w inside {5, 10, 11}, "dist");
      seen_w[d.w] = 1;
      check(d.mode ? (d.u == 3 && d.v == 6) : (d.u >= 20 && d.u <= 22), "implication and if-else");
      check(d.u != d.w, "unique");
    end
    check(!seen_w.exists(7), "dist weight 0");
    ok = d.randomize() with { w == 5; mode == 0; };
    check(ok == 1 && d.w == 5 && d.mode == 0, "randomize() with");
    // no solution: 0, the values stay
    d.x = 42;
    ok = d.randomize() with { x == 7; };
    check(ok == 0 && d.x == 42, "an unsatisfiable call");
    ok = std::randomize(a) with { a > 5; a < 8; };
    check(ok == 1 && (a == 6 || a == 7), "std::randomize");
    if (fails == 0) $display("PASSED");
  end
endmodule
