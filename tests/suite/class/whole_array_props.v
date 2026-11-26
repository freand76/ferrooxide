// Whole fixed-size array properties (IEEE 1800 8.4, 7.6): assigned element by element
// from an array argument of the same name, a pattern, another object's property, and
// copied to an array variable and passed as an array argument; with an enum declared in
// the design (the enum check of the assignment).
typedef enum { RED, GREEN } color_t;

class C;
  color_t col;
  logic [7:0] a [4];
  int b [2:1];
  function new(logic [7:0] a [4]);
    this.a = a;
  endfunction
  function int sum();
    int s = 0;
    foreach (a[i]) s += a[i];
    return s;
  endfunction
endclass

module t;
  integer errors = 0;
  logic [7:0] v [4] = '{1, 2, 3, 4};
  logic [7:0] w [4];
  C c, d;
  function automatic int total(logic [7:0] q [4]);
    return q[0] + q[1] + q[2] + q[3];
  endfunction
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    c = new(v);
    check(`__LINE__, c.a[0], 1);
    check(`__LINE__, c.a[3], 4);
    check(`__LINE__, c.sum(), 10);
    d = new(v);
    c.a = '{10, 20, 30, 40};
    d.a = c.a;
    w = d.a;
    c.b = '{7, 8};
    check(`__LINE__, d.a[1], 20);
    check(`__LINE__, w[2], 30);
    check(`__LINE__, total(c.a), 100);
    check(`__LINE__, c.b[2], 7);
    check(`__LINE__, c.b[1], 8);
    if (errors == 0) $display("PASSED");
  end
endmodule
