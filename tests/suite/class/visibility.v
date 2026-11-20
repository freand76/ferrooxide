// local and protected class members (IEEE 1800-2017 8.18): a local member is visible in its
// own class (also through another object of the class), a protected one also in derived
// classes.
module t;
  class A;
    local int l = 1;
    protected int p = 2;
    local function int lf();
      return l + 10;
    endfunction
    function int sum(A other);
      return l + other.l + lf();
    endfunction
  endclass
  class B extends A;
    function int getp();
      return p * 3;
    endfunction
  endclass
  A a;
  B b;
  initial begin
    a = new;
    b = new;
    if (a.sum(b) === 13 && b.getp() === 6) $display("PASSED");
    else $display("FAILED");
  end
endmodule
