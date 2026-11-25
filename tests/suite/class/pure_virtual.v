// A pure virtual method of an abstract class, implemented in derived classes (IEEE 1800-2017
// 8.21); calls through a base class handle reach the implementation.
module t;
  virtual class Shape;
    pure virtual function int area();
    pure virtual task show();
  endclass
  class Sq extends Shape;
    int s = 3;
    virtual function int area(); return s * s; endfunction
    virtual task show(); $display("square"); endtask
  endclass
  initial begin
    Shape h;
    Sq q = new;
    h = q;
    h.show();
    if (h.area() == 9) $display("PASSED"); else $display("FAILED");
  end
endmodule
