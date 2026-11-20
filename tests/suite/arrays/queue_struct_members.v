// Members of struct elements of queues and dynamic arrays (reads, writes, the iterator of
// array methods), and writes to a part of an element (`q[i][7:0] = v`).
module test;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } P;
  typedef struct { int a; byte b; string s; real r; logic [1:0][7:0] v; } S;
  P pq[$];
  S sq[$];
  S d[];
  S s1;
  int iq[$] = '{32'h12345678};
  int q[$];
  int i = 1;
  int fails = 0;

  task check(input bit ok, input string what);
    if (!ok) begin
      $display("FAILED: %s", what);
      fails++;
    end
  endtask

  // a queue of structs local to an automatic function
  function automatic int f(int x);
    S lq[$];
    lq.push_back(s1);
    lq[0].a = x;
    return lq[0].a + 1;
  endfunction

  initial begin
    // part selects of an element
    iq[0][15:8] = 8'hab;
    iq[i - 1][3 +: 4] = 4'h0;
    check(iq[0] == 32'h1234ab00, "part selects of an element");
    // packed struct elements
    pq.push_back(8'hab);
    check(pq[0].hi == 4'ha && pq[0].lo == 4'hb, "packed struct member reads");
    pq[0].lo = 4'h5;
    check(pq[0] == 8'ha5, "packed struct member write");
    // unpacked struct elements: members of every kind
    s1.a = 5;
    s1.b = -7;
    s1.s = "x";
    s1.r = 1.5;
    s1.v[1] = 8'h3c;
    sq.push_back(s1);
    s1.a = 2;
    s1.s = "yy";
    sq.push_back(s1);
    check(sq[0].a == 5 && sq[0].b == -7 && sq[0].s == "x" && sq[0].r == 1.5 && sq[0].v[1] == 8'h3c, "member reads");
    check(sq[1].a == 2 && sq[1].s == "yy", "member reads of another element");
    sq[i].a = 9;
    sq[1].s = "zzz";
    sq[1].r = 2.25;
    sq[1].v[0] = 8'h11;
    sq[0].b += 1;
    check(sq[1].a == 9 && sq[1].s == "zzz" && sq[1].r == 2.25 && sq[1].v[0] == 8'h11, "member writes");
    check(sq[0].b == -6 && sq[0].a == 5 && sq[0].s == "x", "a member write keeps the other members");
    // the iterator of array methods
    check(sq.sum with (item.a) == 14, "sum of a member");
    q = sq.find_index with (item.s == "zzz");
    check(q.size == 1 && q[0] == 1, "find_index by a string member");
    sq.sort with (-item.a);
    check(sq[0].a == 9 && sq[1].a == 5, "sort by a member");
    // a dynamic array of structs
    d = new[2];
    d[1].a = 3;
    check(d[1].a == 3, "dynamic array element member");
    check(f(41) == 42, "queue of structs in an automatic function");
    if (fails == 0) $display("PASSED");
  end
endmodule
