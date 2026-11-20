// exit: 1
// Using a protected property outside the classes derived from its class (IEEE 1800-2017 8.18).
module test;
  class A;
    local int l = 1;
    protected int p = 2;
    local function int lf();
      return l;
    endfunction
  endclass
  class B extends A;
    function int getl();
      return l;
    endfunction
  endclass
  A a;
  B b;
  initial begin
    a = new;
    b = new;
    $display(b.p);
  end
endmodule
