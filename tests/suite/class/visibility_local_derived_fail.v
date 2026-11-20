// exit: 1
// Using a local property in a derived class (IEEE 1800-2017 8.18).
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
    $display(b.getl());
  end
endmodule
