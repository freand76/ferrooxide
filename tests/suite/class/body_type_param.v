// A `parameter type` or `localparam type` in a class body (IEEE 1800-2017 8.25) names its
// type in the class and, as `C::T`, outside it.
typedef bit [11:0] word_t;
class C;
  parameter type T = byte;
  localparam type U = word_t, V = T;
  T t;
  U u;
  function V twice(V x);
    return x * 2;
  endfunction
endclass
module m;
  C c = new;
  C::U w;
  initial begin
    c.t = 8'h80;
    c.u = 12'hfff;
    w = c.u;
    if ($bits(c.t) == 8 && $bits(C::U) == 12 && $bits(w) == 12 && c.twice(8'd3) == 6
        && c.t < 0)
      $display("PASSED");
    else $display("FAILED: %0d %0d %0d", $bits(c.t), $bits(w), c.twice(8'd3));
  end
endmodule
