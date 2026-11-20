// A specialization of a class declared later (`typedef class C;`), and a class naming its
// own specialization (`typedef C #(T) this_type;`).
typedef class C;
typedef C #(int) CI;

class C #(type KEY = int, T = byte);
  typedef C #(KEY, T) this_type;
  T v;
  this_type next;
  function int w();
    return $bits(v);
  endfunction
endclass

typedef C #(int, shortint) CS;

module test;
  CI a;
  CS b;
  C c;
  initial begin
    a = new;
    b = new;
    c = new;
    b.next = new;
    a.v = 70000;
    b.v = 70000;
    if (a.w() == 8 && b.w() == 16 && b.next.w() == 16 && c.w() == 8 && b.v == 4464 && a.v == 112)
      $display("PASSED");
    else
      $display("FAILED: %0d %0d %0d %0d %0d %0d", a.w(), b.w(), b.next.w(), c.w(), a.v, b.v);
  end
endmodule
