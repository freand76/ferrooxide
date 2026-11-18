// Unpacked structs (IEEE 1800 7.2): members may be real or string; they start as 0.0 and
// "" (7.2.1 default values), other members as x.
typedef struct {
  real        r;
  string      name;
  logic [7:0] b;
  int         n;
} item_t;

typedef struct {
  item_t      it;
  real        scale;
} outer_t;

module t;
  integer errors = 0;
  item_t a, c;
  item_t arr [0:2];
  outer_t o;
  int i;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  task checkr(input integer line, input real got, input real want);
    if (got != want) begin
      $display("FAILED line %0d: got %f want %f", line, got, want);
      errors = errors + 1;
    end
  endtask
  task checks(input integer line, input string got, input string want);
    if (got != want) begin
      $display("FAILED line %0d: got \"%s\" want \"%s\"", line, got, want);
      errors = errors + 1;
    end
  endtask
  function automatic real total(item_t x);
    total = x.r * x.n;
  endfunction
  initial begin
    checkr(`__LINE__, a.r, 0.0);
    checks(`__LINE__, a.name, "");
    check(`__LINE__, $isunknown(a.b), 1);
    a.r = 2.5;
    a.name = "abc";
    a.b = 8'h5A;
    a.n = -3;
    checkr(`__LINE__, a.r, 2.5);
    checkr(`__LINE__, a.r * 2, 5.0);
    checks(`__LINE__, a.name, "abc");
    check(`__LINE__, a.b, 8'h5A);
    check(`__LINE__, a.n, -3);
    checkr(`__LINE__, total(a), -7.5);
    c = a;
    a.name = {a.name, "def"};
    checks(`__LINE__, c.name, "abc");
    checks(`__LINE__, a.name, "abcdef");
    checkr(`__LINE__, c.r, 2.5);
    c = '{r: 1.25, name: "xy", b: 8'h01, n: 7};
    checkr(`__LINE__, c.r, 1.25);
    checks(`__LINE__, c.name, "xy");
    check(`__LINE__, c.n, 7);
    c = '{0.5, "p", 2, 4};
    checkr(`__LINE__, total(c), 2.0);
    checks(`__LINE__, c.name, "p");
    for (i = 0; i < 3; i = i + 1)
      arr[i] = '{r: i * 1.5, name: "e", b: i, n: i};
    arr[1].name = "mid";
    checkr(`__LINE__, arr[2].r, 3.0);
    checks(`__LINE__, arr[1].name, "mid");
    checks(`__LINE__, arr[2].name, "e");
    check(`__LINE__, arr[2].b, 2);
    o.it = a;
    o.scale = 0.5;
    checkr(`__LINE__, o.it.r * o.scale, 1.25);
    checks(`__LINE__, o.it.name, "abcdef");
    if (errors == 0) $display("PASSED");
  end
endmodule
