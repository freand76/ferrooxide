// Associative arrays (IEEE 1800 7.8, 7.9): string and integer keys, num/exists/delete,
// first/last/next/prev, reads of missing keys, copies, foreach in key order.
module t;
  integer errors = 0;
  int ages[string];
  logic [7:0] m[int];
  string names[bit [3:0]];
  int c[string];
  string k;
  int i, n;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    check(`__LINE__, ages.num(), 0);
    ages["bob"] = 30;
    ages["alice"] = 25;
    ages["carol"] = 41;
    check(`__LINE__, ages.num(), 3);
    check(`__LINE__, ages.size(), 3);
    check(`__LINE__, ages["alice"], 25);
    check(`__LINE__, ages.exists("bob"), 1);
    check(`__LINE__, ages.exists("dave"), 0);
    // a missing key reads as the default value
    check(`__LINE__, ages["dave"], 0);
    check(`__LINE__, m[7], 8'hxx);
    // keys in order
    check(`__LINE__, ages.first(k), 1);
    if (k != "alice") begin $display("FAILED line %0d: first %s", `__LINE__, k); errors++; end
    check(`__LINE__, ages.next(k), 1);
    if (k != "bob") begin $display("FAILED line %0d: next %s", `__LINE__, k); errors++; end
    check(`__LINE__, ages.last(k), 1);
    if (k != "carol") begin $display("FAILED line %0d: last %s", `__LINE__, k); errors++; end
    check(`__LINE__, ages.next(k), 0);
    if (k != "carol") begin $display("FAILED line %0d: unchanged %s", `__LINE__, k); errors++; end
    check(`__LINE__, ages.prev(k), 1);
    if (k != "bob") begin $display("FAILED line %0d: prev %s", `__LINE__, k); errors++; end
    // a copy
    c = ages;
    ages.delete("bob");
    check(`__LINE__, ages.num(), 2);
    check(`__LINE__, c.num(), 3);
    check(`__LINE__, c["bob"], 30);
    n = 0;
    foreach (c[key]) n += c[key];
    check(`__LINE__, n, 96);
    // integer keys, negative ones first
    m[10] = 8'hA;
    m[-3] = 8'h3;
    m[2] = 8'h2;
    check(`__LINE__, m.first(i), 1);
    check(`__LINE__, i, -3);
    n = 0;
    foreach (m[j]) n = n * 100 + j;
    check(`__LINE__, n, -3 * 10000 + 2 * 100 + 10);
    m.delete();
    check(`__LINE__, m.num(), 0);
    names[4'hF] = "f";
    names[4'h1] = "one";
    check(`__LINE__, names.last(i[3:0]), 1);
    check(`__LINE__, i[3:0], 4'hF);
    if (names[1] != "one") begin $display("FAILED line %0d", `__LINE__); errors++; end
    if (errors == 0) $display("PASSED");
  end
endmodule
