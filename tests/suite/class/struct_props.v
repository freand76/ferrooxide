// Class properties of struct types (IEEE 1800 8.4, 7.2), also given by a type parameter
// (8.25): members read and written through a handle and inside methods, parts of members,
// assignment patterns, real and string members (starting as 0.0 and ""), struct
// arguments and results through a type parameter (13.4), chains of handles.
typedef struct packed { logic [7:0] a; logic signed [3:0] b; } s_t;
typedef struct { string name; real r; logic [3:0] n; } u_t;

class C #(type T = logic);
  s_t s;
  T t;
  u_t u;
  function new(T x);
    s = x;
    t = x;
  endfunction
  function int get();
    return s.a + t.b;
  endfunction
  function void set(logic [3:0] v);
    s.b = v;
    t.a[1:0] = v[1:0];
  endfunction
  function T add(T y);
    T z;
    z = y;
    z.a = t.a + y.a;
    return z;
  endfunction
endclass

class H;
  C #(s_t) c;
  s_t s;
endclass

module t;
  integer errors = 0;
  C #(s_t) c;
  H h;
  s_t r;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    c = new('{8'h12, 4'h3});
    check(`__LINE__, c.s, 12'h123);
    check(`__LINE__, c.s.a, 8'h12);
    check(`__LINE__, c.t.b, 3);
    check(`__LINE__, c.get(), 8'h15);
    c.set(4'h9);
    check(`__LINE__, c.s, 12'h129);
    check(`__LINE__, c.t, 12'h113);
    c.s.a = 8'hff;
    c.t.b[0] = 1'b0;
    check(`__LINE__, c.s, 12'hff9);
    check(`__LINE__, c.t, 12'h112);
    // a signed member reads as signed
    c.t.b = -4'sd3;
    if (!(c.t.b < 0)) begin
      $display("FAILED line %0d: signed member", `__LINE__);
      errors = errors + 1;
    end
    r = c.add('{8'h01, 4'h7});
    check(`__LINE__, r, 12'h127);
    // an unpacked struct with real and string members
    if (c.u.name != "" || c.u.r != 0.0 || c.u.n !== 4'bx) begin
      $display("FAILED line %0d: start values", `__LINE__);
      errors = errors + 1;
    end
    c.u.name = "abc";
    c.u.r = 1.5;
    c.u.n = 7;
    if (c.u.name != "abc" || c.u.r != 1.5 || c.u.n != 7) begin
      $display("FAILED line %0d: members", `__LINE__);
      errors = errors + 1;
    end
    c.u = '{"x", 2.5, 3};
    if (c.u.name != "x" || c.u.r != 2.5 || c.u.n != 3) begin
      $display("FAILED line %0d: pattern", `__LINE__);
      errors = errors + 1;
    end
    // through a chain of handles
    h = new;
    h.c = c;
    h.c.t.a = 8'h55;
    h.s = '{a: 8'h66, b: 4'h1};
    check(`__LINE__, c.t.a, 8'h55);
    check(`__LINE__, h.c.t, 12'h55d);
    check(`__LINE__, h.s.a, 8'h66);
    if (errors == 0) $display("PASSED");
  end
endmodule
