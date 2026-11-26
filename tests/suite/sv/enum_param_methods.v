// Enum methods of a variable whose enum a type parameter gives (IEEE
// 1800-2017 6.19.5), also in a static method of a parameterized class.
typedef enum {A, B = 5, C} abc_e;

class names #(type T = abc_e);
  static string all;
  static function void collect();
    T e = e.first();
    all = "";
    do begin
      all = {all, e.name(), " "};
      e = e.next();
    end while (e != e.first());
  endfunction
endclass

module m #(type T = int);
  T e;
  initial begin
    e = e.first();
    if (e.name() != "A" || e.num() != 3) $display("FAILED: first, name, num");
    e = e.next(2);
    if (e != C || e.prev() != B || e.last() != C) $display("FAILED: next, prev, last");
    e = e.next();
    if (e != A) $display("FAILED: next wraps");
    names#(T)::collect();
    if (names#(T)::all == "A B C ") $display("PASSED");
    else $display("FAILED: '%s'", names#(T)::all);
  end
endmodule

module top;
  m #(abc_e) i();
endmodule
