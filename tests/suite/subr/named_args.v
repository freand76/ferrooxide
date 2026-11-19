// Arguments bound by name (IEEE 1800-2017 13.5.4): in any order, after positional ones,
// with left-out and empty (`.b()`) ones taking their defaults (13.5.3); also for tasks and
// class methods.
module test;
  reg failed = 0;

  function automatic integer f(integer a, integer b = 10, integer c = 100);
    return a + 2 * b + 3 * c;
  endfunction

  task automatic t(input integer x, output integer y, input integer z = 5);
    y = x * z;
  endtask

  class C;
    integer base = 1000;
    function integer m(integer p, integer q = 7);
      return base + p - q;
    endfunction
  endclass

  initial begin
    integer r;
    C o = new;
    if (f(.c(1), .a(2), .b(3)) !== 2 + 6 + 3) failed = 1;
    if (f(1, .c(2)) !== 1 + 20 + 6) failed = 1;
    if (f(.a(4)) !== 4 + 20 + 300) failed = 1;
    if (f(.a(4), .b()) !== 4 + 20 + 300) failed = 1;
    if (f(5, 6, .c(7)) !== 5 + 12 + 21) failed = 1;
    t(.y(r), .x(3));
    if (r !== 15) failed = 1;
    t(2, .z(4), .y(r));
    if (r !== 8) failed = 1;
    if (o.m(.q(1), .p(2)) !== 1001) failed = 1;
    if (o.m(.p(9)) !== 1002) failed = 1;
    if (failed) $display("FAILED");
    else $display("PASSED");
  end
endmodule
