// constraint prototypes (IEEE 1800-2017 18.5.1) with their bodies after the class, an
// implicit prototype without one (an empty constraint), pure constraints (18.5.2)
// overridden in a derived class
virtual class Base;
  rand int unsigned v;
  pure constraint range_c;
endclass

class Item extends Base;
  rand int unsigned w;
  extern constraint w_c;
  constraint range_c;
  constraint empty_c;
endclass

constraint Item::w_c { w inside {[10:20]}; w > v; }
constraint Item::range_c { v < 15; }

module top;
  Item it;
  int fails = 0;
  initial begin
    it = new;
    repeat (200) begin
      if (!it.randomize()) fails++;
      if (it.v >= 15 || it.w < 10 || it.w > 20 || it.w <= it.v) fails++;
    end
    if (fails == 0) $display("PASSED");
    else $display("FAILED %0d", fails);
  end
endmodule
