// classes: properties, constructors, methods through this/super, new, null, method calls
class Base;
  int n = 1;
  function new(int k = 2);
    n = k;
  endfunction
  virtual function int get();
    return n;
  endfunction
endclass

class Derived extends Base(5);
  string s;
  task run(int n);
    s = "x";
    this.n = n + get();
    super.n = 0;
    void_call();
  endtask
  function void void_call();
  endfunction
endclass

module m;
  Derived d;
  Base b;
  initial begin
    d = new;
    b = new d;
    d.run(3);
    if (d != null && d.get() == 1) d.n = d.s.len();
    d.void_call;
    b = null;
  end
endmodule
