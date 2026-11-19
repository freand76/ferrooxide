// Members and methods through a class handle reached by a hierarchical name
// (`inst.obj.m()`, `inst.obj.p`; IEEE 1800-2017 23.6, 8.4): reads, writes, method calls,
// task calls, each instance with its own class (a class declared in a module).
module M #(parameter X = 0);
  class C;
    int x = X;
    function int f;
      return x + 1;
    endfunction
    task t(int v);
      x = v;
    endtask
  endclass
  C c = new;
endmodule

module t;
  integer errors = 0;
  M #(10) m1();
  M #(20) m2();
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    #1;
    check(`__LINE__, m1.c.f(), 11);
    check(`__LINE__, m2.c.x, 20);
    check(`__LINE__, m1.c.x + m2.c.f(), 31);
    m1.c.t(5);
    m2.c.x = 7;
    check(`__LINE__, m1.c.x, 5);
    check(`__LINE__, m2.c.f(), 8);
    if (errors == 0) $display("PASSED");
  end
endmodule
