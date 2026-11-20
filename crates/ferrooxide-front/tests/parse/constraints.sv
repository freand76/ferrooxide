// constraints, rand properties, randomize() forms, randcase
class B;
  rand int x;
  randc bit [3:0] k;
  rand byte arr[4];
  int lim;
  constraint c1 { x > 0; soft x < lim; }
  constraint c2 { x inside {1, [3:5]}; x dist {1 := 2, [3:5] :/ 1}; }
  constraint c3 { lim > 2 -> x == 1; if (lim == 0) x != 2; else { x != 3; } }
  constraint c4 { foreach (arr[i]) arr[i] == i; unique {x, lim}; solve x before lim; }
  function int f(int y);
    return randomize() with { x < y; };
  endfunction
endclass
module m;
  B b;
  int a, ok;
  initial begin
    ok = b.randomize(x) with { x > local::a; };
    b.randomize();
    ok = std::randomize(a) with { a inside {[1:3]}; };
    randcase
      1: a = 1;
      a + 1: a = 2;
    endcase
  end
endmodule
