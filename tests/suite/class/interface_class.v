// interface classes (IEEE 1800-2017 8.26): handles of an interface class reach objects of
// the classes implementing it, calls choose the object's implementation, casts between
// interface classes check the object's class
module top;
  interface class Shape;
    typedef int unsigned area_t;
    pure virtual function area_t area();
    pure virtual function string name();
  endclass
  interface class Scalable;
    pure virtual function void scale(int k);
  endclass
  interface class Solid extends Shape;
    pure virtual function int volume(int depth);
  endclass
  interface class Box #(type T = int);
    pure virtual function T get();
  endclass

  class Rect implements Shape, Scalable;
    int w, h;
    function new(int w, int h);
      this.w = w;
      this.h = h;
    endfunction
    virtual function Shape::area_t area();
      return w * h;
    endfunction
    virtual function string name();
      return "rect";
    endfunction
    virtual function void scale(int k);
      w *= k;
      h *= k;
    endfunction
  endclass
  class Square extends Rect;
    function new(int s);
      super.new(s, s);
    endfunction
    virtual function string name();
      return "square";
    endfunction
  endclass
  // a partial implementation: the derived class completes it
  virtual class Base implements Solid;
    virtual function Shape::area_t area();
      return 3;
    endfunction
    pure virtual function string name();
  endclass
  class Cube extends Base implements Box #(byte);
    virtual function string name();
      return "cube";
    endfunction
    virtual function int volume(int depth);
      return area() * depth;
    endfunction
    virtual function byte get();
      return 8'd42;
    endfunction
  endclass

  Shape sh[3];
  Scalable sc;
  Solid so;
  Box #(byte) bx;
  Rect r;
  Cube c;
  int fails = 0;
  string names;
  initial begin
    r = new(2, 3);
    sh[0] = r;
    begin
      Square q = new(4);
      sh[1] = q;
    end
    c = new;
    sh[2] = c;
    names = "";
    foreach (sh[i]) names = {names, sh[i].name(), " "};
    if (names != "rect square cube ") fails++;
    if (sh[0].area() != 6 || sh[1].area() != 16 || sh[2].area() != 3) fails++;
    // an interface handle to another interface of the same object
    if (!$cast(sc, sh[1])) fails++;
    sc.scale(2);
    if (sh[1].area() != 64) fails++;
    if ($cast(sc, sh[2])) fails++;
    if (!$cast(so, sh[2]) || so.volume(5) != 15) fails++;
    if ($cast(so, sh[0])) fails++;
    bx = c;
    if (bx.get() != 42) fails++;
    if (fails == 0)
      $display("PASSED");
    else
      $display("FAILED %0d (%s)", fails, names);
  end
endmodule
