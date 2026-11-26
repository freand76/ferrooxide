// Associative arrays with keys wider than 64 bits and with packed struct keys (IEEE
// 1800-2017 7.8.4: integral keys of any width, in numeric order; a packed struct is
// integral), also as class properties and in foreach loops whose key variable has the
// struct type.
typedef struct packed { bit [63:0] lo; bit [63:0] hi; int unsigned s; } range_t;

class M;
  string by[range_t];
  bit [99:0] w[bit [99:0]];
endclass

module t;
  integer errors = 0;
  string a[range_t];
  int sw[logic signed [79:0]];
  range_t r;
  M m;
  string got;
  bit [99:0] k;
  task check(input integer line, input bit ok);
    if (!ok) begin
      $display("FAILED line %0d", line);
      errors = errors + 1;
    end
  endtask
  initial begin
    r.lo = 5; r.hi = 9; r.s = 1;
    a[r] = "x";
    r.lo = 1;
    a[r] = "y";
    check(`__LINE__, a.num() == 2 && a[r] == "y");
    got = "";
    foreach (a[i]) got = {got, $sformatf("%0d:%0d:%s ", i.lo, i.hi, a[i])};
    check(`__LINE__, got == "1:9:y 5:9:x ");
    m = new;
    m.by[r] = "z";
    got = "";
    foreach (m.by[i]) got = {got, $sformatf("%0d:%0d:%s ", i.lo, i.s, m.by[i])};
    check(`__LINE__, got == "1:1:z ");
    // wide keys in numeric order; first/next through a wide key variable
    m.w[100'h1_0000_0000_0000_0000_0000] = 1;
    m.w[3] = 2;
    m.w[100'h2_0000_0000_0000_0000] = 3;
    got = "";
    foreach (m.w[i]) got = {got, $sformatf("%0d ", m.w[i])};
    check(`__LINE__, got == "2 3 1 ");
    check(`__LINE__, m.w.first(k) && k == 3);
    check(`__LINE__, m.w.next(k) && k == 100'h2_0000_0000_0000_0000);
    check(`__LINE__, m.w.exists(100'h1_0000_0000_0000_0000_0000));
    check(`__LINE__, !m.w.exists(4));
    // signed wide keys: negative ones first
    sw[-80'sd5] = 1;
    sw[80'sd7] = 2;
    sw[-80'sd1] = 3;
    got = "";
    foreach (sw[i]) got = {got, $sformatf("%0d ", i)};
    check(`__LINE__, got == "-5 -1 7 ");
    if (errors == 0) $display("PASSED");
  end
endmodule
