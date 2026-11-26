// A class whose base class is declared after it (with a forward typedef,
// IEEE 1800-2017 8.27): the derived class has the base class's properties
// and methods, also a parameterized derived class.
module test;
  typedef class B;
  class D extends B;
    int y = 2;
    function int sum(); return x + y + base_f(); endfunction
  endclass
  class P #(type T = int) extends B;
    T z = 5;
  endclass
  class B;
    int x = 1;
    function int base_f(); return 10; endfunction
  endclass
  initial begin
    D d = new;
    P #(byte) p = new;
    if (d.sum() == 13 && d.x == 1 && p.x == 1 && p.z == 5) $display("PASSED");
    else $display("FAILED");
  end
endmodule
