// A static rand property (IEEE 1800-2017 18.4) is shared by all objects of the class:
// randomize() of any of them gives it a new value under the constraints.
class A;
  static rand int b;
  rand bit [3:0] x;
  constraint c { b > 5; b < 12; x == b; }
endclass
module t;
  A a1 = new, a2 = new;
  int bad = 0;
  initial begin
    repeat (20) begin
      if (!a1.randomize()) bad++;
      if (!(A::b inside {[6:11]}) || a2.b != A::b || a1.x != A::b) bad++;
      if (!a2.randomize() with { b == 7; }) bad++;
      if (a1.b != 7 || a2.x != 7) bad++;
    end
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %0d", bad);
  end
endmodule
