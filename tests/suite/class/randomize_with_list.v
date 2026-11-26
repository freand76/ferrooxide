// randomize() with an identifier list (IEEE 1800-2017 18.7.1): only the names listed are
// the object's in the constraints, the others the caller's
class A;
  rand int x;
  int y = -1;
endclass

module top;
  function automatic int f(A obj, int y);
    return obj.randomize() with (x) { x > 0; x < y; };
  endfunction
  A a;
  int fails = 0;
  initial begin
    a = new;
    repeat (50) begin
      if (f(a, 5) != 1 || a.x <= 0 || a.x >= 5) fails++;
    end
    // without the list, y is the object's: no solution
    if (a.randomize() with { x > 0; x < y; }) fails++;
    if (fails == 0) $display("PASSED");
    else $display("FAILED %0d", fails);
  end
endmodule
