// A rand class handle property (IEEE 1800-2017 18.4): randomize() of the object also
// randomizes the object the handle refers to, under that object's constraints; a null
// handle is skipped.
class Inner;
  rand bit [7:0] v;
  int pre = 0, post = 0;
  constraint c { v inside {[10:20]}; }
  function void pre_randomize(); pre++; endfunction
  function void post_randomize(); post++; endfunction
endclass
class Base;
  rand bit [3:0] b;
endclass
class Outer extends Base;
  rand bit [3:0] x;
  rand Inner in1, in2;
  Inner plain;
  constraint c { x < 4; }
  function new(); in1 = new; plain = new; endfunction
endclass
module t;
  Outer o = new;
  int bad = 0;
  initial begin
    repeat (20) begin
      if (!o.randomize()) bad++;
      if (o.x >= 4 || o.in1.v < 10 || o.in1.v > 20) bad++;
      if (o.plain.v != 0) bad++;
    end
    if (o.in1.pre != 20 || o.in1.post != 20) bad++;
    // a handle set later takes part; the inner objects keep their values on a failure
    o.in2 = new;
    if (!o.randomize() || o.in2.v < 10 || o.in2.v > 20) bad++;
    begin
      bit [7:0] v = o.in2.v;
      if (o.randomize() with { x > 5; }) bad++;
      if (o.in2.v != v) bad++;
    end
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %0d", bad);
  end
endmodule
