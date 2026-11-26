// Queues and associative arrays of unpacked structs as class properties (IEEE 1800-2017
// 7.10, 8.4): elements and their members read and written through a handle and inside
// methods, foreach; a property whose type, given by a type parameter, is a queue.
typedef struct { int a; string s; } p_t;

class C;
  p_t q[$];
  p_t m[string];
  function void add(int a, string s);
    p_t x;
    x.a = a;
    x.s = s;
    q.push_back(x);
  endfunction
  function string first();
    return q[0].s;
  endfunction
endclass

class Shared #(type T = int);
  T value;
endclass

typedef int iq_t[$];

module t;
  integer errors = 0;
  C c;
  p_t y;
  string got;
  Shared #(iq_t) sh;
  task check(input integer line, input bit ok);
    if (!ok) begin
      $display("FAILED line %0d", line);
      errors = errors + 1;
    end
  endtask
  initial begin
    c = new;
    c.add(3, "hi");
    c.add(4, "ho");
    c.q[1].s = "yo";
    c.q[0].a = 9;
    y = c.q[1];
    check(`__LINE__, c.q.size() == 2 && c.q[1].a == 4 && c.q[0].s == "hi" && c.q[1].s == "yo");
    check(`__LINE__, c.first() == "hi" && c.q[0].a == 9 && y.s == "yo");
    got = "";
    foreach (c.q[i]) got = {got, $sformatf("%0d%s ", c.q[i].a, c.q[i].s)};
    check(`__LINE__, got == "9hi 4yo ");
    c.m["k"] = y;
    c.m["k"].a = 11;
    check(`__LINE__, c.m["k"].a == 11 && c.m["k"].s == "yo" && y.a == 4);
    sh = new;
    sh.value.push_back(5);
    sh.value.push_back(6);
    check(`__LINE__, sh.value.size() == 2 && sh.value[1] == 6);
    if (errors == 0) $display("PASSED");
  end
endmodule
