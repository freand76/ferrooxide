// forward typedefs (IEEE 1800 6.18): the type may be used before its definition in the same
// scope; repeated forward declarations are allowed, also after the definition
module t;
  integer errors = 0;
  typedef T1;
  typedef T1;
  typedef enum E;
  typedef struct S;
  typedef union U;
  typedef B;
  T1 x = -1;
  E e;
  S s;
  U u;
  B b4;
  typedef enum B {P, Q} E2;     // a forward type as an enum base type
  E2 e2;
  typedef integer T1;
  typedef T1;
  typedef enum logic [2:0] {A, C = 5} E;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } S;
  typedef union packed { logic [7:0] w; S s; } U;
  typedef logic [5:0] B;
  typedef string Str;
  Str name = "abc";
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    check(`__LINE__, $bits(x), 32);
    check(`__LINE__, x, -1);
    check(`__LINE__, $bits(e), 3);
    e = C;
    check(`__LINE__, e, 5);
    s = 8'h3c;
    check(`__LINE__, s.hi, 3);
    u.w = 8'ha5;
    check(`__LINE__, u.s.lo, 5);
    check(`__LINE__, $bits(b4), 6);
    check(`__LINE__, $bits(e2), 6);
    check(`__LINE__, name.len(), 3);
    name = Str'(16'h4869);              // a cast to a string type: "Hi"
    if (name != "Hi") begin $display("FAILED line %0d: %s", `__LINE__, name); errors++; end
    if (errors == 0) $display("PASSED");
  end
endmodule
