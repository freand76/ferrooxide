// `soft a -> b;` is a soft constraint on the implication `a -> b` (IEEE 1800-2017
// 18.5.13.1, 11.4.7): it holds unless a hard constraint contradicts it.
class C;
  rand bit mode;
  rand int length;
  bit force_short;
  constraint deflt {
    soft length inside {32, 1024};
    soft mode -> length == 1024;
  }
  constraint short_c { force_short -> length == 32; }
endclass
module t;
  C c = new;
  int bad = 0;
  initial begin
    repeat (20) begin
      if (!c.randomize()) bad++;
      if (!(c.length inside {32, 1024})) bad++;
      if (c.mode && c.length != 1024) bad++;
    end
    c.force_short = 1;
    repeat (20) begin
      if (!c.randomize() with { mode == 1; }) bad++;
      if (c.length != 32 || !c.mode) bad++;
    end
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %0d", bad);
  end
endmodule
