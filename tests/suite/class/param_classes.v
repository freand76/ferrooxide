// Parameterized classes (IEEE 1800-2017 8.25): value and type parameters, defaults,
// specializations as types, base classes and in `C #(..)::x` / `C #(..)::new`, equal
// values naming one class, parameters without a default.
module test;
  class base_cls #(int b = 20);
    int a;
    function int get_b();
      return b;
    endfunction
  endclass

  // a base class specialized by the class's own parameter
  class ext_cls #(int e = 25) extends base_cls #(e + 1);
    function int get_e();
      return e;
    endfunction
  endclass

  class par_cls #(int a = 25);
    parameter int b = a + 1;
    function int f();
      return par_cls::b * 2;  // (inside the class: this specialization)
    endfunction
  endclass

  class super_cls;
    int s = 2;
    function new(int def = 3);
      s = def;
    endfunction
  endclass

  class test_cls #(int t = 12) extends super_cls;
    int a;
    function new(int def = 42);
      super.new(def + 3);
      a = def - t;
    endfunction
  endclass

  class box #(type T = int);
    T v;
    function T get();
      return v;
    endfunction
    function void set(T x);
      v = x;
    endfunction
  endclass

  class need #(int N, type T);
    T v;
    function int n();
      return N;
    endfunction
  endclass

  localparam int K = 15;
  ext_cls #(K) inst;
  ext_cls d;
  super_cls super_obj;
  test_cls #(.t(23)) tc;
  box #(byte) bb;
  box #(string) bs;
  box bi;
  box #(int) bi2;
  box #() bi3;
  need #(3, byte) nd;
  par_cls #(1) p;
  int fails = 0;

  task check(input bit ok, input string what);
    if (!ok) begin
      $display("FAILED: %s", what);
      fails++;
    end
  endtask

  initial begin
    inst = new;
    d = new;
    check(inst.get_e() == 15 && inst.get_b() == 16, "a specialization and its base");
    check(d.get_e() == 25 && d.get_b() == 26, "the defaults");
    check(par_cls#()::b == 26 && par_cls#(5)::b == 6, "C #(..)::P");
    p = new;
    check(p.f() == 4, "C::P inside the class");
    super_obj = test_cls#(.t(23))::new(.def(41));
    check(super_obj.s == 44, "C #(..)::new with named arguments");
    tc = new(30);
    check(tc.a == 7, "named parameter value");
    bb = new;
    bb.set(300);
    check(bb.get() == 44 && $bits(bb.v) == 8, "type parameter byte");
    bs = new;
    bs.set("hi");
    check(bs.get() == "hi", "type parameter string");
    bi = new;
    bi.set(300);
    check(bi.get() == 300, "type parameter default");
    // the same values: the same class (handles assign)
    bi2 = bi;
    bi3 = bi2;
    check(bi3.get() == 300, "equal parameter values name one class");
    nd = new;
    nd.v = 255;
    check(nd.n() == 3 && nd.v == -1, "parameters without a default");
    if (fails == 0) $display("PASSED");
  end
endmodule
