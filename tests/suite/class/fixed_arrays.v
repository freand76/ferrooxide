// Fixed-size unpacked arrays of class handles (IEEE 1800 7.4, 8.4) as variables and as
// class properties, also of other element types: element reads and writes through a handle
// and inside methods, foreach, members of elements, invalid indices (7.4.6: reads give the
// default value, x here; writes do nothing), shallow copies (8.12).
class E;
  int v;
  function new(int x);
    v = x;
  endfunction
endclass

class C;
  logic [7:0] m[4];
  int g[1:0][0:2];
  real r[2];
  string s[3:1];
  E e[0:1][0:2];
  function new();
    foreach (g[i, j]) g[i][j] = i * 10 + j;
    foreach (e[i, j]) e[i][j] = new(i * 3 + j);
  endfunction
  function int sum();
    sum = 0;
    foreach (g[i, j]) sum += g[i][j];
  endfunction
endclass

module t;
  integer errors = 0;
  C c, d;
  E ea[3], eb[0:1][2:0];
  int i, n;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    // arrays of handles as variables: every element starts as null
    check(`__LINE__, ea[1] == null, 1);
    ea[0] = new(3);
    ea[2] = ea[0];
    ea[2].v = 5;
    check(`__LINE__, ea[0].v, 5);
    eb[1][0] = new(7);
    check(`__LINE__, eb[1][0].v, 7);
    n = 0;
    foreach (eb[k, l]) if (eb[k][l] != null) n++;
    check(`__LINE__, n, 1);
    // array properties
    c = new;
    check(`__LINE__, c.m[0], 8'hxx);
    check(`__LINE__, c.sum(), 36);
    c.m[2] = 8'h5a;
    i = 2;
    check(`__LINE__, c.m[i], 8'h5a);
    check(`__LINE__, c.m[i + 5], 8'hxx);
    c.m[7] = 1;
    check(`__LINE__, {c.m[0], c.m[1], c.m[2], c.m[3]}, 32'hxxxx5axx);
    check(`__LINE__, c.g[1][2], 12);
    check(`__LINE__, c.g[0][1], 1);
    c.r[1] = 1.5;
    if (c.r[0] != 0.0 || c.r[1] != 1.5) begin
      $display("FAILED line %0d", `__LINE__);
      errors = errors + 1;
    end
    c.s[3] = "x";
    if (c.s[3] != "x" || c.s[1] != "") begin
      $display("FAILED line %0d", `__LINE__);
      errors = errors + 1;
    end
    check(`__LINE__, c.e[1][2].v, 5);
    check(`__LINE__, c.e[0][1].v, 1);
    // a shallow copy copies the arrays (the objects they refer to are shared)
    d = new c;
    d.g[1][2] = 99;
    check(`__LINE__, c.g[1][2], 12);
    check(`__LINE__, d.g[1][2], 99);
    d.e[1][1].v = 42;
    check(`__LINE__, c.e[1][1].v, 42);
    c.e[0][0] = null;
    check(`__LINE__, c.e[0][0] == null, 1);
    check(`__LINE__, d.e[0][0] == null, 0);
    if (errors == 0) $display("PASSED");
  end
endmodule
