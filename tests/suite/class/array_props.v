// Queues, dynamic and associative arrays as class properties (IEEE 1800 8.4, 7.5, 7.8,
// 7.10): methods and elements through a handle and inside methods, copies on assignment
// (7.6) and shallow copies (8.12), bounded queues, separate arrays per object.
class C;
  int q[$];
  real r[];
  string s[string];
  logic [3:0] v[$:2];
  function void add(int x);
    q.push_back(x);
  endfunction
  function int last();
    return q[$];
  endfunction
endclass

module t;
  integer errors = 0;
  C a, b;
  int d, t[];
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    a = new;
    check(`__LINE__, a.q.size(), 0);
    a.q.push_back(4);
    a.q.push_front(3);
    a.add(7);
    check(`__LINE__, a.q.size(), 3);
    check(`__LINE__, a.q[0], 3);
    check(`__LINE__, a.q[2], 7);
    check(`__LINE__, a.last(), 7);
    d = a.q.pop_front();
    check(`__LINE__, d, 3);
    check(`__LINE__, a.q.size, 2);
    a.q[0] = 9;
    a.q.insert(1, 5);
    check(`__LINE__, a.q[0], 9);
    check(`__LINE__, a.q[1], 5);
    check(`__LINE__, a.q[$], 7);
    a.q.delete(0);
    check(`__LINE__, a.q.size(), 2);
    check(`__LINE__, a.q[0], 5);
    // dynamic array of reals
    a.r = new[2];
    a.r[1] = 2.5;
    check(`__LINE__, a.r.size(), 2);
    if (a.r[1] != 2.5) begin
      $display("FAILED line %0d", `__LINE__);
      errors = errors + 1;
    end
    // assignments copy the array
    t = '{1, 2, 3};
    a.q = t;
    t[0] = 8;
    check(`__LINE__, a.q[0], 1);
    check(`__LINE__, a.q.size(), 3);
    t = a.q;
    a.q[0] = 6;
    check(`__LINE__, t[0], 1);
    check(`__LINE__, a.q[0], 6);
    // associative array with string keys
    a.s["x"] = "y";
    if (a.s["x"] != "y") begin
      $display("FAILED line %0d", `__LINE__);
      errors = errors + 1;
    end
    check(`__LINE__, a.s.num(), 1);
    check(`__LINE__, a.s.exists("z"), 0);
    // a bounded queue keeps at most 3 elements (a warning for the fourth)
    a.v.push_back(1);
    a.v.push_back(2);
    a.v.push_back(3);
    check(`__LINE__, a.v.size(), 3);
    // another object has its own arrays
    b = new;
    check(`__LINE__, b.q.size(), 0);
    // a shallow copy has arrays of its own (8.12)
    b = new a;
    b.q[0] = 1;
    b.s["x"] = "z";
    check(`__LINE__, a.q[0], 6);
    check(`__LINE__, b.q[0], 1);
    if (a.s["x"] != "y" || b.s["x"] != "z") begin
      $display("FAILED line %0d", `__LINE__);
      errors = errors + 1;
    end
    a.q.delete();
    check(`__LINE__, a.q.size(), 0);
    check(`__LINE__, b.q.size(), 3);
    if (errors == 0) $display("PASSED");
  end
endmodule
