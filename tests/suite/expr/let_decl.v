// expected
// let declarations (IEEE 1800-2017 11.12): positional and named arguments, defaults, typed
// formals (the actual is cast to the type), untyped ones, a let without arguments and
// lets that use others.
module top;
  logic [3:0] a = 12, b = 15, c = 7;
  logic d;
  let op(x, y, z) = |((x | y) & z);
  let sum(byte p, byte q = 2) = p + q;
  let twice(untyped v) = v + v;
  let five = 5;
  let nest(x) = twice(x) + five;
  int x = 100;
  initial begin
    #1;
    d = op(.x(a), .y(b), .z(c));
    $display("%b %0d %0d %0d %0d %0d", d, op(a, 4'h0, 4'h0), sum(250, 10), sum(3), twice(a), five);
    $display("%0d %0d", nest(3), nest(x));
    $display("%0d", $bits(sum(1, 2)));
  end
endmodule
